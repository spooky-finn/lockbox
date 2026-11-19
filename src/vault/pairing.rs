//! Durable link commands orchestrate existing addressed invitation APIs.
//! `receive` authenticates inbound messages; `delivery` owns the durable outbox
//! and worker publication plans. This module owns local policy and progression.

use crate::{
    sync::{
        protocol::{
            Error, Hash, Machine, Role, canonical,
            pairing::{
                ClosedReason, Closure, Contact, Envelope, Kind, LinkReference, Slot, Ticket,
            },
        },
        transport::{pairing::MailboxFactory, storj::pairing::Factory},
    },
    vault::{
        InvitationStatus, Result, Vault, VaultError,
        profile::pairing::{Approval, IssuedLink, JoiningLink, PairingState, ResolvingLink},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod delivery;
mod receive;

#[cfg(test)]
#[path = "tests/pairing.rs"]
mod tests;

impl Vault {
    pub(crate) fn issued_links(&self) -> impl Iterator<Item = &IssuedLink> {
        self.profile
            .as_ref()
            .into_iter()
            .flat_map(|profile| profile.pairing.issued.values())
    }

    pub(crate) fn joining_link(&self) -> Option<&JoiningLink> {
        let links = &self.profile.as_ref()?.pairing.joining;
        links
            .values()
            .find(|link| !link.joined && link.closed.is_none())
            .or_else(|| links.values().next_back())
    }

    pub(crate) fn has_pairing(&self) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|profile| !profile.pairing.is_empty())
    }

    pub(crate) fn resolving_link(&self) -> Option<&ResolvingLink> {
        self.profile.as_ref()?.pairing.resolving.as_ref()
    }

    pub(crate) fn link_ready(&self, session: Hash) -> bool {
        let Some(link) = self
            .issued_links()
            .find(|link| link.ticket.body.id == session)
        else {
            return false;
        };
        let Ok(hash) = link.ticket.hash() else {
            return false;
        };
        link.closed.is_none()
            && [Slot::Bootstrap, Slot::Ticket(hash)]
                .into_iter()
                .all(|slot| {
                    self.profile
                        .as_ref()
                        .unwrap()
                        .pairing
                        .outgoing
                        .iter()
                        .any(|delivery| {
                            delivery.session == session
                                && delivery.slot == slot
                                && delivery.published
                        })
                })
    }

    pub(crate) fn export_link(&mut self, session: Hash, code: bool) -> Result<String> {
        self.ensure_profile_writable()?;
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| profile.pairing.needs_save)
        {
            self.save_pairing()?;
        }
        let link = self
            .issued_links()
            .find(|link| link.ticket.body.id == session)
            .ok_or(Error::InvalidInvitation)?;
        if link.closed.is_some() {
            return Err(Error::InvitationUsed.into());
        }
        if code {
            Ok(link.ticket.verification_code()?.to_string())
        } else {
            if !self.link_ready(session) {
                return Err(VaultError::Pairing(
                    "Wait for the invitation to finish uploading",
                ));
            }
            Ok(link.ticket.reference()?.link())
        }
    }

    pub(crate) fn dismiss_join_link(&mut self, session: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
        if self
            .resolving_link()
            .is_some_and(|link| link.reference.ticket == session)
        {
            self.profile.as_mut().unwrap().pairing.resolving = None;
            return self.save_pairing();
        }
        let link = self
            .profile
            .as_ref()
            .and_then(|profile| profile.pairing.joining.get(&session))
            .ok_or(Error::InvalidInvitation)?;
        if link.joined || (link.consented && link.closed.is_none()) {
            return Err(Error::InvitationUsed.into());
        }
        let state = &mut self.profile.as_mut().unwrap().pairing;
        state.joining.remove(&session);
        state
            .outgoing
            .retain(|delivery| delivery.session != session);
        self.save_pairing()
    }

    pub(crate) fn pairing_waits_to_join(&self) -> bool {
        self.profile.as_ref().is_some_and(|profile| {
            profile.pairing.resolving.is_some()
                || profile
                    .pairing
                    .joining
                    .values()
                    .any(|link| !link.joined && link.closed.is_none())
        })
    }

    pub(crate) fn create_link(&mut self, now: u64, days: u64) -> Result<Hash> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        if !matches!(days, 1 | 7 | 30) {
            return Err(Error::InvalidInvitation.into());
        }
        let expires = now
            .checked_add(days * 86400)
            .ok_or(Error::InvalidInvitation)?;
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Self::pairing_capacity(&profile.pairing)?;
        let ticket = Machine::pairing_ticket(
            profile.sync.as_ref().ok_or(Error::UnauthorizedDevice)?,
            &profile.sync_secret(),
            &profile.provider.base_url,
            &profile.provider.bucket,
            &profile.space_name,
            expires,
        )?;
        let proof = Machine::pairing_proof(profile.sync.as_ref().unwrap(), &ticket)?;
        let id = ticket.body.id;
        let hash = ticket.hash()?;
        let bytes = canonical(&ticket)?;
        let profile = self.profile.as_mut().unwrap();
        profile.pairing.issued.insert(
            id,
            IssuedLink {
                ticket,
                requests: BTreeMap::new(),
                rejected: BTreeSet::new(),
                approval: None,
                closed: None,
            },
        );
        self.queue_pairing(id, Slot::Bootstrap, proof)?;
        self.queue_pairing(id, Slot::Ticket(hash), bytes)?;
        self.save_pairing()?;
        Ok(id)
    }

    pub(crate) fn request_link(&mut self, link: &str, code: &str, now: u64) -> Result<Hash> {
        self.ensure_profile_writable()?;
        self.ensure_sharing_view()?;
        let supplied = code
            .trim()
            .to_ascii_lowercase()
            .parse::<Hash>()
            .map_err(|_| {
                VaultError::Pairing(
                    "Enter the administrator's verification code from a second channel",
                )
            })?;
        if LinkReference::is_short(link) {
            let reference = LinkReference::parse(link)?;
            let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
            if let Some(pending) = &profile.pairing.resolving {
                if pending.reference == reference && pending.code == supplied {
                    self.save_pairing()?;
                    return Ok(reference.ticket);
                }
                return Err(VaultError::Pairing(
                    "Finish or cancel the current connection first",
                ));
            }
            if profile
                .pairing
                .joining
                .values()
                .any(|link| !link.joined && link.closed.is_none())
            {
                return Err(VaultError::Pairing(
                    "Finish or cancel the current connection first",
                ));
            }
            Self::pairing_capacity(&profile.pairing)?;
            let hash = reference.ticket;
            self.profile.as_mut().unwrap().pairing.resolving = Some(ResolvingLink {
                reference,
                code: supplied,
            });
            self.save_pairing()?;
            return Ok(hash);
        }
        if self.resolving_link().is_some() {
            return Err(VaultError::Pairing(
                "Finish or cancel the current connection first",
            ));
        }
        self.stage_link_ticket(Ticket::parse_link(link)?, supplied, now)
    }

    fn stage_link_ticket(&mut self, ticket: Ticket, supplied: Hash, now: u64) -> Result<Hash> {
        if supplied != ticket.verification_code()? {
            return Err(VaultError::Pairing(
                "The administrator's verification code does not match the link",
            ));
        }
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Self::check_pairing_storage(profile, &ticket)?;
        let id = ticket.body.id;
        if let Some(existing) = profile.pairing.joining.get(&id) {
            if existing.ticket.hash()? != ticket.hash()? {
                return Err(Error::InvalidInvitation.into());
            }
            self.profile.as_mut().unwrap().pairing.resolving = None;
            self.save_pairing()?;
            return Ok(id);
        }
        if now >= ticket.body.expires_at.0 {
            return Err(Error::InvitationExpired.into());
        }
        if profile
            .pairing
            .joining
            .values()
            .any(|joining| !joining.joined && joining.closed.is_none())
        {
            return Err(VaultError::Pairing(
                "Finish or cancel the current connection first",
            ));
        }
        if profile.pairing.resolving.is_none() {
            Self::pairing_capacity(&profile.pairing)?;
        }
        self.ensure_joinable(&ticket.body.descriptor)?;
        let pairing = &mut self.profile.as_mut().unwrap().pairing;
        pairing.resolving = None;
        pairing.joining.insert(
            id,
            JoiningLink {
                ticket,
                proof_verified: false,
                request: None,
                offer: None,
                consented: false,
                joined: false,
                closed: None,
            },
        );
        self.save_pairing()?;
        Ok(id)
    }

    pub(crate) fn approve_link_request(
        &mut self,
        session: Hash,
        request: Hash,
        role: Role,
        verified: bool,
        now: u64,
    ) -> Result<()> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        if !verified {
            return Err(VaultError::Pairing(
                "Compare both user and device IDs through a second channel first",
            ));
        }
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let link = profile
            .pairing
            .issued
            .get(&session)
            .ok_or(Error::InvalidInvitation)?;
        if link.closed.is_some() {
            return Err(Error::InvitationUsed.into());
        }
        if now >= link.ticket.body.expires_at.0 {
            return Err(Error::InvitationExpired.into());
        }
        if link.ticket.body.descriptor
            != self.space_descriptor().ok_or(Error::UnauthorizedDevice)?
            || self.space_controller()? != profile.sync_secret().id()
        {
            return Err(Error::InsufficientRole.into());
        }
        let contact = link
            .requests
            .get(&request)
            .ok_or(Error::InvalidInvitation)?;
        contact.verify_contact(&link.ticket)?;
        let members = self.space_members()?;
        if members.iter().any(|(certificate, current_role)| {
            certificate.body.user == contact.body.certificate.body.user && *current_role != role
        }) || Machine::was_member(
            self.sync_state().unwrap(),
            contact.body.certificate.body.device,
        )? {
            return Err(Error::InvalidInvitation.into());
        }
        if link.rejected.contains(&request)
            || link
                .approval
                .as_ref()
                .is_some_and(|approval| approval.request != request || approval.role != role)
        {
            return Err(Error::InvitationUsed.into());
        }
        if role == Role::Owner
            && contact.body.certificate.body.user != link.ticket.body.controller.body.user
        {
            return Err(Error::InsufficientRole.into());
        }
        let user = contact.body.certificate.body.user;
        let name = contact.body.username.clone();
        let profile = self.profile.as_mut().unwrap();
        profile
            .pairing
            .issued
            .get_mut(&session)
            .unwrap()
            .approval
            .get_or_insert(Approval {
                request,
                role,
                invitation: None,
            });
        profile.participant_names.entry(user).or_insert(name);
        // Reserve the exact peer/role before any offer is issued or delivered.
        self.save_pairing()?;
        self.advance_pairing(now)
    }

    pub(crate) fn reject_link_request(&mut self, session: Hash, request: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
        let link = self
            .profile
            .as_ref()
            .and_then(|profile| profile.pairing.issued.get(&session))
            .ok_or(Error::InvalidInvitation)?
            .clone();
        if link.rejected.contains(&request) {
            return Ok(());
        }
        if link
            .approval
            .as_ref()
            .is_some_and(|approval| approval.request == request)
        {
            return Err(Error::InvitationUsed.into());
        }
        let contact = link
            .requests
            .get(&request)
            .ok_or(Error::InvalidInvitation)?;
        let closure = Closure::create(
            &link.ticket,
            ClosedReason::Rejected,
            Some(request),
            &self.profile.as_ref().unwrap().sync_secret(),
        )?;
        let frame = Envelope::seal(
            &link.ticket,
            request,
            Kind::Rejected,
            &contact.body.certificate.body,
            &closure,
        )?;
        self.profile
            .as_mut()
            .unwrap()
            .pairing
            .issued
            .get_mut(&session)
            .unwrap()
            .rejected
            .insert(request);
        self.queue_pairing(session, Slot::Rejected(request), frame.bytes()?)?;
        self.save_pairing()
    }

    pub(crate) fn cancel_link(&mut self, session: Hash) -> Result<()> {
        self.close_link(session, ClosedReason::Cancelled)
    }

    pub(crate) fn can_forget_link(&self, session: Hash) -> bool {
        self.profile.as_ref().is_some_and(|profile| {
            profile
                .pairing
                .issued
                .get(&session)
                .is_some_and(|link| link.closed.is_some())
                && profile
                    .pairing
                    .outgoing
                    .iter()
                    .filter(|delivery| delivery.session == session)
                    .all(|delivery| delivery.published)
        })
    }

    pub(crate) fn forget_link(&mut self, session: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
        if !self.can_forget_link(session) {
            return Err(Error::PendingIntent.into());
        }
        let pairing = &mut self.profile.as_mut().unwrap().pairing;
        pairing.issued.remove(&session);
        pairing
            .outgoing
            .retain(|delivery| delivery.session != session);
        self.save_pairing()
    }

    fn close_link(&mut self, session: Hash, reason: ClosedReason) -> Result<()> {
        self.ensure_profile_writable()?;
        let link = self
            .profile
            .as_ref()
            .and_then(|profile| profile.pairing.issued.get(&session))
            .ok_or(Error::InvalidInvitation)?
            .clone();
        if link.closed.is_some() {
            return Ok(());
        }
        if reason != ClosedReason::Used
            && let Some(invitation) = link
                .approval
                .as_ref()
                .and_then(|approval| approval.invitation)
        {
            self.cancel_invitation(invitation)?;
        }
        let closure = Closure::create(
            &link.ticket,
            reason,
            None,
            &self.profile.as_ref().unwrap().sync_secret(),
        )?;
        let bytes = canonical(&closure)?;
        self.profile
            .as_mut()
            .unwrap()
            .pairing
            .issued
            .get_mut(&session)
            .unwrap()
            .closed = Some(closure);
        // No old offer or receipt may be newly published after terminal policy commits.
        for delivery in &mut self.profile.as_mut().unwrap().pairing.outgoing {
            if delivery.session == session {
                delivery.published = true;
            }
        }
        self.queue_pairing(session, Slot::Status, bytes)?;
        self.save_pairing()
    }

    pub(crate) fn consent_link(&mut self, session: Hash, now: u64) -> Result<()> {
        self.ensure_profile_writable()?;
        let joining = self
            .profile
            .as_ref()
            .and_then(|profile| profile.pairing.joining.get(&session))
            .ok_or(Error::InvalidInvitation)?
            .clone();
        if joining.closed.is_some() || joining.joined {
            return Err(Error::InvitationUsed.into());
        }
        if !joining.consented && now >= joining.ticket.body.expires_at.0 {
            return Err(Error::InvitationExpired.into());
        }
        let offer = joining.offer.as_ref().ok_or(Error::MissingDependency)?;
        Machine::verify_pairing_invitation(&joining.ticket, offer)?;
        let consent = self.accept_invitation(offer.clone(), &joining.ticket.body.descriptor)?;
        let request = joining
            .request
            .as_ref()
            .ok_or(Error::InvalidInvitation)?
            .hash()?;
        if !self.has_delivery(session, Slot::Consent(request)) {
            let envelope = Envelope::seal(
                &joining.ticket,
                request,
                Kind::Consent,
                &joining.ticket.body.controller.body,
                &consent,
            )?;
            self.queue_pairing(session, Slot::Consent(request), envelope.bytes()?)?;
        }
        self.profile
            .as_mut()
            .unwrap()
            .pairing
            .joining
            .get_mut(&session)
            .unwrap()
            .consented = true;
        self.save_pairing()
    }

    pub(crate) fn advance_pairing(&mut self, now: u64) -> Result<()> {
        self.ensure_profile_writable()?;
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| profile.pairing.needs_save)
        {
            self.save_pairing()?;
        }
        self.recover_sync()?;
        let issued: Vec<_> = self
            .profile
            .as_ref()
            .map(|profile| profile.pairing.issued.values().cloned().collect())
            .unwrap_or_default();
        for link in issued {
            if link.closed.is_some() {
                continue;
            }
            let reserved = link
                .approval
                .as_ref()
                .and_then(|approval| approval.invitation)
                .and_then(|id| self.invitation_status(id, now).ok())
                .is_some_and(|status| {
                    matches!(
                        status,
                        InvitationStatus::Admitting | InvitationStatus::Admitted
                    )
                });
            if now >= link.ticket.body.expires_at.0 && !reserved {
                self.close_link(link.ticket.body.id, ClosedReason::Expired)?;
                continue;
            }
            let hash = link.ticket.hash()?;
            if !self.has_delivery(link.ticket.body.id, Slot::Ticket(hash)) {
                self.queue_pairing(
                    link.ticket.body.id,
                    Slot::Ticket(hash),
                    canonical(&link.ticket)?,
                )?;
                self.save_pairing()?;
            }
            if let Some(approval) = &link.approval {
                if self.has_delivery(link.ticket.body.id, Slot::Offer(approval.request)) {
                    continue;
                }
                let request = &link.requests[&approval.request];
                let offer = match approval.invitation {
                    Some(id) => self.export_invitation(id)?,
                    None => {
                        let offer = self.issue_invitation(
                            request.body.certificate.clone(),
                            approval.role,
                            Some(link.ticket.body.expires_at.0),
                        )?;
                        self.profile
                            .as_mut()
                            .unwrap()
                            .pairing
                            .issued
                            .get_mut(&link.ticket.body.id)
                            .unwrap()
                            .approval
                            .as_mut()
                            .unwrap()
                            .invitation = Some(offer.id());
                        self.save_pairing()?;
                        offer
                    }
                };
                if !self.has_delivery(link.ticket.body.id, Slot::Offer(approval.request)) {
                    let frame = Envelope::seal(
                        &link.ticket,
                        approval.request,
                        Kind::Offer,
                        &request.body.certificate.body,
                        &offer,
                    )?;
                    self.queue_pairing(
                        link.ticket.body.id,
                        Slot::Offer(approval.request),
                        frame.bytes()?,
                    )?;
                    self.save_pairing()?;
                }
            }
        }
        let joining: Vec<_> = self
            .profile
            .as_ref()
            .map(|profile| profile.pairing.joining.values().cloned().collect())
            .unwrap_or_default();
        for link in joining {
            if link.closed.is_none()
                && !link.joined
                && !link.consented
                && now >= link.ticket.body.expires_at.0
            {
                let state = &mut self.profile.as_mut().unwrap().pairing;
                state.joining.get_mut(&link.ticket.body.id).unwrap().closed =
                    Some(ClosedReason::Expired);
                for delivery in &mut state.outgoing {
                    if delivery.session == link.ticket.body.id {
                        delivery.published = true;
                    }
                }
                self.save_pairing()?;
                continue;
            }
            if link.closed.is_some()
                || link.joined
                || !link.proof_verified
                || link.request.is_some()
            {
                continue;
            }
            let profile = self.profile.as_ref().unwrap();
            let contact = Contact::create(
                &link.ticket,
                &profile.username,
                profile
                    .certificate
                    .clone()
                    .ok_or(Error::UnauthorizedDevice)?,
                &profile.sync_secret(),
            )?;
            let request = contact.hash()?;
            let frame = Envelope::seal(
                &link.ticket,
                request,
                Kind::Request,
                &link.ticket.body.controller.body,
                &contact,
            )?;
            self.queue_pairing(link.ticket.body.id, Slot::Request(request), frame.bytes()?)?;
            self.profile
                .as_mut()
                .unwrap()
                .pairing
                .joining
                .get_mut(&link.ticket.body.id)
                .unwrap()
                .request = Some(contact);
            self.save_pairing()?;
        }
        Ok(())
    }

    pub(crate) fn pairing_factory(&self, ticket: &Ticket) -> Result<Arc<dyn MailboxFactory>> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Self::check_pairing_storage(profile, ticket)?;
        self.pairing_storage_factory(&ticket.body.space_name)
    }

    pub(crate) fn pairing_storage_factory(
        &self,
        space_name: &str,
    ) -> Result<Arc<dyn MailboxFactory>> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        if !crate::sync::protocol::pairing::valid_name(space_name) {
            return Err(Error::InvalidInvitation.into());
        }
        let config = &profile.provider;
        Ok(Arc::new(Factory::new(
            &config.base_url,
            &config.bucket,
            space_name,
            &config.access_key,
            &config.secret_key,
        )))
    }

    fn check_pairing_storage(
        profile: &super::profile::BootstrapProfile,
        ticket: &Ticket,
    ) -> Result<()> {
        let config = &profile.provider;
        let configured = url::Url::parse(&config.base_url).map_err(|_| Error::InvalidInvitation)?;
        let target =
            url::Url::parse(&ticket.body.endpoint).map_err(|_| Error::InvalidInvitation)?;
        if configured != target || config.bucket != ticket.body.bucket {
            return Err(VaultError::Pairing(
                "This link uses different storage; configure the administrator's endpoint and bucket first",
            ));
        }
        Ok(())
    }

    fn pairing_capacity(state: &PairingState) -> Result<()> {
        if state.issued.len() + state.joining.len() + usize::from(state.resolving.is_some())
            >= PairingState::MAX_LINKS
        {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        Ok(())
    }
}
