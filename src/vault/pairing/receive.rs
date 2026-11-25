//! Authenticate bounded mailbox input before applying invitation transitions.
//! Admission and durable commits still pass through the vault facade.

use crate::{
    sync::protocol::{
        Admission, Error, Hash, Invitation, Machine,
        pairing::{ClosedReason, Closure, Contact, Envelope, Joined, Kind, Slot},
    },
    vault::{Result, Vault, profile::pairing::PairingState},
};

impl Vault {
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
        if let Slot::Ticket(hash) = slot {
            let pending = self
                .resolving_link()
                .ok_or(Error::InvalidInvitation)?
                .clone();
            if hash != session || pending.reference.ticket != hash {
                return Err(Error::InvalidInvitation.into());
            }
            let ticket = pending.reference.decode_ticket(bytes)?;
            self.stage_link_ticket(ticket, pending.code, now)?;
            return Ok(());
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
                if link.managed.is_none() && !self.has_delivery(session, Slot::Receipt(request)) {
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
                let (receipt, provider) = Self::managed_receipt(&link, request, &frame, &device)?;
                if Some(receipt.id()) != link.offer.as_ref().map(Invitation::id) {
                    return Err(Error::InvalidInvitation.into());
                }
                self.verify_join_receipt(&receipt)?;
                self.join_with_storage(receipt.clone(), provider)?;
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
}
