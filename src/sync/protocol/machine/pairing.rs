//! Bootstrap proof checks for contact links; membership still requires addressed consent.

use super::{
    Machine, SyncState,
    history::with_history,
    invitations::{encode_history, metadata_state},
};
use crate::sync::protocol::{
    Blob, Counter, DeviceSecret, Error, Hash, Invitation, Result, canonical, crypto,
    pairing::{TICKET_DOMAIN, Ticket, TicketBody, proof_hash},
};

impl Machine {
    pub(crate) fn pairing_ticket(
        state: &SyncState,
        device: &DeviceSecret,
        endpoint: &str,
        bucket: &str,
        space_name: &str,
        expires_at: u64,
    ) -> Result<Ticket> {
        state.require_idle()?;
        Self::replay_state(state, device)?.require_ready()?;
        with_history(state, |history| {
            let (anchor, current) = history.tip();
            if current.body.controller != device.id() {
                return Err(Error::InsufficientRole);
            }
            let certificate = current
                .body
                .members
                .iter()
                .find(|member| member.certificate.body.device == device.id())
                .ok_or(Error::UnauthorizedDevice)?
                .certificate
                .clone();
            let proof = canonical(&encode_history(history)?)?;
            let ticket = crypto::sign(
                TicketBody {
                    id: Hash(*crypto::random32()?),
                    descriptor: state.descriptor(),
                    anchor,
                    proof: proof_hash(&proof),
                    controller: certificate,
                    endpoint: endpoint.to_owned(),
                    bucket: bucket.to_owned(),
                    space_name: space_name.to_owned(),
                    expires_at: Counter(expires_at),
                },
                &device.signer(),
                TICKET_DOMAIN,
            )?;
            ticket.verify_ticket()?;
            ticket.link()?;
            Ok(ticket)
        })
    }

    pub(crate) fn pairing_proof(state: &SyncState, ticket: &Ticket) -> Result<Vec<u8>> {
        if state.descriptor() != ticket.body.descriptor {
            return Err(Error::InvalidInvitation);
        }
        with_history(state, |history| {
            let index = history
                .chain
                .iter()
                .position(|(hash, _)| *hash == ticket.body.anchor)
                .ok_or(Error::MissingDependency)?;
            let bytes: Vec<_> = history.chain[..=index]
                .iter()
                .map(|(_, control)| {
                    crate::sync::protocol::Object::Control((*control).clone())
                        .bytes()
                        .map(Blob)
                })
                .collect::<Result<_>>()?;
            let bytes = canonical(&bytes)?;
            if proof_hash(&bytes) != ticket.body.proof {
                return Err(Error::InvalidInvitation);
            }
            Ok(bytes)
        })
    }

    pub(crate) fn verify_pairing_proof(ticket: &Ticket, bytes: &[u8]) -> Result<()> {
        ticket.verify_ticket()?;
        if proof_hash(bytes) != ticket.body.proof {
            return Err(Error::HashMismatch);
        }
        let proof: Vec<Blob> = crate::sync::protocol::wire::decode_bounded(
            bytes,
            crate::sync::protocol::MAX_INVITATION_BYTES,
        )?;
        let state = metadata_state(&ticket.body.descriptor, &proof)?;
        with_history(&state, |history| {
            let (head, control) = history.tip();
            if head != ticket.body.anchor
                || history.chain.len() != state.objects.len()
                || control.body.controller != ticket.body.controller.body.device
                || !control.body.members.iter().any(|member| {
                    member.certificate.hash().ok() == ticket.body.controller.hash().ok()
                })
            {
                return Err(Error::InvalidInvitation);
            }
            Ok(())
        })
    }

    pub(crate) fn verify_pairing_invitation(
        ticket: &Ticket,
        invitation: &Invitation,
    ) -> Result<()> {
        Self::verify_invitation(invitation, &ticket.body.descriptor)?;
        let state = metadata_state(&ticket.body.descriptor, &invitation.history)?;
        with_history(&state, |history| {
            if !history.contains(ticket.body.anchor) {
                return Err(Error::MissingDependency);
            }
            Ok(())
        })
    }
}
