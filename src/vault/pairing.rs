//! Durable link sessions orchestrate existing addressed invitation APIs.

use crate::{
    sharing::transport::MailboxFactory,
    sync::{
        protocol::{
            Admission, AdmissionReceipt, Blob, Error, Hash, Invitation, Machine, Role, canonical,
            pairing::{ClosedReason, Closure, Contact, Envelope, Joined, Kind, Ticket},
        },
        transport::storj::pairing::Factory,
    },
    vault::{
        InvitationStatus, Result, Vault, VaultError,
        profile::pairing::{Approval, Delivery, IssuedLink, JoiningLink, PairingState, Slot},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct MailPlan {
    pub ticket: Ticket,
    pub outgoing: Vec<Delivery>,
    pub probes: Vec<Slot>,
    pub list_requests: bool,
    pub known_requests: BTreeSet<Hash>,
}

#[cfg(test)]
#[path = "tests/pairing.rs"]
mod tests;

impl Vault {
    pub(crate) fn issued_links(&self) -> impl Iterator<Item = &IssuedLink> {
        self.profile
            .iter()
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
            link.ticket.link().map_err(Into::into)
        }
    }

    pub(crate) fn dismiss_join_link(&mut self, session: Hash) -> Result<()> {
        self.ensure_profile_writable()?;
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
            profile
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
        self.save_pairing()?;
        Ok(id)
    }

    pub(crate) fn request_link(&mut self, link: &str, code: &str, now: u64) -> Result<Hash> {
        self.ensure_profile_writable()?;
        self.ensure_sharing_view()?;
        let ticket = Ticket::parse_link(link)?;
        let supplied = code
            .trim()
            .to_ascii_lowercase()
            .parse::<Hash>()
            .map_err(|_| {
                VaultError::Pairing(
                    "Enter the administrator's verification code from a second channel",
                )
            })?;
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
        Self::pairing_capacity(&profile.pairing)?;
        self.ensure_joinable(&ticket.body.descriptor)?;
        self.profile.as_mut().unwrap().pairing.joining.insert(
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

    pub(crate) fn receive_pairing(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        now: u64,
    ) -> Result<()> {
        self.ensure_profile_writable()?;
        if bytes.len() > slot.budget() {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| profile.pairing.issued.contains_key(&session))
        {
            self.receive_link_request(session, slot, bytes, now)
        } else {
            self.receive_link_reply(session, slot, bytes, now)
        }
    }

    fn receive_link_request(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        now: u64,
    ) -> Result<()> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let link = profile
            .pairing
            .issued
            .get(&session)
            .ok_or(Error::InvalidInvitation)?
            .clone();
        if link.closed.is_some() {
            return Err(Error::InvitationUsed.into());
        }
        let device = profile.sync_secret();
        let frame = Envelope::decode(bytes)?;
        match slot {
            Slot::Request(request) => {
                if link.approval.is_some() || now >= link.ticket.body.expires_at.0 {
                    return Err(Error::InvitationUsed.into());
                }
                let contact: Contact = frame.open(&link.ticket, request, Kind::Request, &device)?;
                contact.verify_contact(&link.ticket)?;
                if contact.hash()? != request {
                    return Err(Error::InvalidInvitation.into());
                }
                if link.requests.contains_key(&request) {
                    return Ok(());
                }
                if link.requests.len() >= PairingState::MAX_REQUESTS {
                    return Err(Error::ResourceBudgetExceeded.into());
                }
                self.profile
                    .as_mut()
                    .unwrap()
                    .pairing
                    .issued
                    .get_mut(&session)
                    .unwrap()
                    .requests
                    .insert(request, contact);
                self.save_pairing()
            }
            Slot::Consent(request) => {
                let approval = link
                    .approval
                    .as_ref()
                    .filter(|approval| approval.request == request)
                    .ok_or(Error::InvalidInvitation)?;
                let consent: Admission =
                    frame.open(&link.ticket, request, Kind::Consent, &device)?;
                let contact = link
                    .requests
                    .get(&request)
                    .ok_or(Error::InvalidInvitation)?;
                if Some(consent.id()) != approval.invitation
                    || consent.role() != approval.role
                    || consent.recipient().hash()? != contact.body.certificate.hash()?
                {
                    return Err(Error::InvalidInvitation.into());
                }
                self.verify_invitation_response(&consent, now)?;
                let receipt = self.admit_invitation(consent, now)?;
                if !self.has_delivery(session, Slot::Receipt(request)) {
                    let frame = Envelope::seal(
                        &link.ticket,
                        request,
                        Kind::Receipt,
                        &contact.body.certificate.body,
                        &receipt,
                    )?;
                    self.queue_pairing(session, Slot::Receipt(request), frame.bytes()?)?;
                }
                self.save_pairing()
            }
            Slot::Joined(request) => {
                let approval = link
                    .approval
                    .as_ref()
                    .filter(|approval| approval.request == request)
                    .ok_or(Error::InvalidInvitation)?;
                let invitation = approval.invitation.ok_or(Error::InvalidInvitation)?;
                let receipt = self
                    .profile
                    .as_ref()
                    .unwrap()
                    .sharing
                    .issued
                    .get(&invitation)
                    .and_then(|record| record.receipt.as_ref())
                    .ok_or(Error::InvalidInvitation)?;
                let ack: Joined = frame.open(&link.ticket, request, Kind::Joined, &device)?;
                ack.verify_joined(
                    &link.ticket,
                    &link.requests[&request],
                    invitation,
                    receipt.control(),
                )?;
                self.close_link(session, ClosedReason::Used)
            }
            _ => Err(Error::InvalidInvitation.into()),
        }
    }

    fn receive_link_reply(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: &[u8],
        now: u64,
    ) -> Result<()> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        let link = profile
            .pairing
            .joining
            .get(&session)
            .ok_or(Error::InvalidInvitation)?
            .clone();
        if link.closed.is_some() || link.joined {
            return Ok(());
        }
        let device = profile.sync_secret();
        if slot == Slot::Status {
            let closure: Closure = crate::sync::protocol::decode(bytes)?;
            closure.verify_closure(&link.ticket)?;
            if closure.body.request.is_some() || closure.body.reason == ClosedReason::Rejected {
                return Err(Error::InvalidInvitation.into());
            }
            self.profile
                .as_mut()
                .unwrap()
                .pairing
                .joining
                .get_mut(&session)
                .unwrap()
                .closed = Some(closure.body.reason);
            return self.save_pairing();
        }
        if slot == Slot::Bootstrap {
            if now >= link.ticket.body.expires_at.0 {
                return Err(Error::InvitationExpired.into());
            }
            Machine::verify_pairing_proof(&link.ticket, bytes)?;
            self.profile
                .as_mut()
                .unwrap()
                .pairing
                .joining
                .get_mut(&session)
                .unwrap()
                .proof_verified = true;
            self.save_pairing()?;
            return self.advance_pairing(now);
        }
        let request = link
            .request
            .as_ref()
            .ok_or(Error::MissingDependency)?
            .hash()?;
        let frame = Envelope::decode(bytes)?;
        match slot {
            Slot::Offer(id) if id == request => {
                let offer: Invitation = frame.open(&link.ticket, request, Kind::Offer, &device)?;
                Machine::verify_pairing_invitation(&link.ticket, &offer)?;
                if offer.recipient().hash()?
                    != self
                        .device_certificate()
                        .ok_or(Error::UnauthorizedDevice)?
                        .hash()?
                    || offer.expires_at() != Some(link.ticket.body.expires_at.0)
                    || offer.offer.body.author != link.ticket.body.controller.body.device
                    || link
                        .offer
                        .as_ref()
                        .is_some_and(|previous| previous.id() != offer.id())
                {
                    return Err(Error::InvalidInvitation.into());
                }
                if link.offer.is_none() {
                    self.profile
                        .as_mut()
                        .unwrap()
                        .pairing
                        .joining
                        .get_mut(&session)
                        .unwrap()
                        .offer = Some(offer);
                    self.save_pairing()?;
                }
                Ok(())
            }
            Slot::Receipt(id) if id == request && link.consented => {
                let receipt: AdmissionReceipt =
                    frame.open(&link.ticket, request, Kind::Receipt, &device)?;
                if Some(receipt.id()) != link.offer.as_ref().map(Invitation::id) {
                    return Err(Error::InvalidInvitation.into());
                }
                self.verify_join_receipt(&receipt)?;
                self.join_invitation(receipt.clone())?;
                let ack = Joined::create(
                    &link.ticket,
                    request,
                    receipt.id(),
                    receipt.control(),
                    &device,
                )?;
                let frame = Envelope::seal(
                    &link.ticket,
                    request,
                    Kind::Joined,
                    &link.ticket.body.controller.body,
                    &ack,
                )?;
                self.queue_pairing(session, Slot::Joined(request), frame.bytes()?)?;
                self.profile
                    .as_mut()
                    .unwrap()
                    .pairing
                    .joining
                    .get_mut(&session)
                    .unwrap()
                    .joined = true;
                self.save_pairing()
            }
            Slot::Rejected(id) if id == request => {
                let rejected: Closure =
                    frame.open(&link.ticket, request, Kind::Rejected, &device)?;
                rejected.verify_closure(&link.ticket)?;
                if rejected.body.request != Some(request)
                    || rejected.body.reason != ClosedReason::Rejected
                {
                    return Err(Error::InvalidInvitation.into());
                }
                self.profile
                    .as_mut()
                    .unwrap()
                    .pairing
                    .joining
                    .get_mut(&session)
                    .unwrap()
                    .closed = Some(ClosedReason::Rejected);
                self.save_pairing()
            }
            _ => Err(Error::InvalidInvitation.into()),
        }
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

    pub(crate) fn pairing_plans(&mut self, now: u64) -> Result<Vec<MailPlan>> {
        self.advance_pairing(now)?;
        // Persist/reconcile a previous ambiguous save before any bytes can leave this process.
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| profile.pairing.needs_save)
        {
            self.save_pairing()?;
        }
        let Some(profile) = &self.profile else {
            return Ok(Vec::new());
        };
        let mut plans = Vec::new();
        for link in profile.pairing.issued.values() {
            let mut probes = Vec::new();
            if link.closed.is_none()
                && let Some(approval) = &link.approval
            {
                probes.push(Slot::Consent(approval.request));
                probes.push(Slot::Joined(approval.request));
            }
            plans.push(MailPlan {
                ticket: link.ticket.clone(),
                outgoing: self.pending_deliveries(link.ticket.body.id),
                probes,
                list_requests: link.closed.is_none() && link.approval.is_none(),
                known_requests: link.requests.keys().copied().collect(),
            });
        }
        for link in profile.pairing.joining.values() {
            let mut probes = Vec::new();
            if link.closed.is_none() && !link.joined {
                probes.push(Slot::Status);
                if !link.proof_verified {
                    probes.push(Slot::Bootstrap);
                } else if let Some(request) = &link.request {
                    let request = request.hash()?;
                    probes.push(Slot::Rejected(request));
                    if link.offer.is_none() {
                        probes.push(Slot::Offer(request));
                    } else if link.consented {
                        probes.push(Slot::Receipt(request));
                    }
                }
            }
            plans.push(MailPlan {
                ticket: link.ticket.clone(),
                outgoing: self.pending_deliveries(link.ticket.body.id),
                probes,
                list_requests: false,
                known_requests: BTreeSet::new(),
            });
        }
        plans.retain(|plan| {
            plan.list_requests || !plan.probes.is_empty() || !plan.outgoing.is_empty()
        });
        Ok(plans)
    }

    pub(crate) fn acknowledge_pairing(&mut self, session: Hash, slots: &[Slot]) -> Result<()> {
        if slots.is_empty() {
            return Ok(());
        }
        self.ensure_profile_writable()?;
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        for delivery in &mut profile.pairing.outgoing {
            if delivery.session == session && slots.contains(&delivery.slot) {
                delivery.published = true;
            }
        }
        self.save_pairing()
    }

    pub(crate) fn retry_pairing(&mut self) -> Result<()> {
        self.ensure_profile_writable()?;
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        for delivery in &mut profile.pairing.outgoing {
            let active = profile
                .pairing
                .issued
                .get(&delivery.session)
                .is_some_and(|link| link.closed.is_none())
                || profile
                    .pairing
                    .joining
                    .get(&delivery.session)
                    .is_some_and(|link| link.closed.is_none());
            if active {
                delivery.published = false;
            }
        }
        self.save_pairing()
    }

    pub(crate) fn pairing_factory(&self, ticket: &Ticket) -> Result<Arc<dyn MailboxFactory>> {
        let profile = self.profile.as_ref().ok_or(Error::UnauthorizedDevice)?;
        Self::check_pairing_storage(profile, ticket)?;
        let config = &profile.provider;
        Ok(Arc::new(Factory::new(
            &config.base_url,
            &config.bucket,
            &ticket.body.space_name,
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
        if state.issued.len() + state.joining.len() >= PairingState::MAX_LINKS {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        Ok(())
    }

    fn has_delivery(&self, session: Hash, slot: Slot) -> bool {
        self.profile.as_ref().is_some_and(|profile| {
            profile
                .pairing
                .outgoing
                .iter()
                .any(|delivery| delivery.session == session && delivery.slot == slot)
        })
    }

    fn pending_deliveries(&self, session: Hash) -> Vec<Delivery> {
        self.profile
            .as_ref()
            .into_iter()
            .flat_map(|profile| profile.pairing.outgoing.iter())
            .filter(|delivery| delivery.session == session && !delivery.published)
            .cloned()
            .collect()
    }

    fn queue_pairing(&mut self, session: Hash, slot: Slot, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() > slot.budget() {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        let pairing = &mut self
            .profile
            .as_mut()
            .ok_or(Error::UnauthorizedDevice)?
            .pairing;
        if let Some(previous) = pairing
            .outgoing
            .iter()
            .find(|delivery| delivery.session == session && delivery.slot == slot)
        {
            if previous.bytes.0 != bytes {
                return Err(Error::InvalidInvitation.into());
            }
            return Ok(());
        }
        if pairing.outgoing.len() >= 256
            || crate::serialization::canonical_size(pairing).map_err(|_| Error::InvalidEncoding)?
                + bytes.len() * 4 / 3
                + 1024
                > 64 * 1024 * 1024
        {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        pairing.outgoing.push(Delivery {
            session,
            slot,
            bytes: Blob(bytes),
            published: false,
        });
        Ok(())
    }

    fn save_pairing(&mut self) -> Result<()> {
        let profile = self.profile.as_mut().ok_or(Error::UnauthorizedDevice)?;
        profile.pairing.needs_save = true;
        profile.enable_pairing();
        if !profile.pairing.validate() {
            return Err(Error::ResourceBudgetExceeded.into());
        }
        self.save_inner(false)?;
        self.profile.as_mut().unwrap().pairing.needs_save = false;
        Ok(())
    }
}
