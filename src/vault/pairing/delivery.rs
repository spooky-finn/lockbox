//! Durable mailbox outbox, probe planning and acknowledgements.
//! Reconcile ambiguous saves before exposing publication plans to workers.

use crate::{
    sync::protocol::{
        Blob, Error, Hash,
        pairing::{Slot, Ticket},
    },
    vault::{Result, Vault, profile::pairing::Delivery},
};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(crate) struct MailPlan {
    pub ticket: Ticket,
    pub outgoing: Vec<Delivery>,
    pub probes: Vec<Slot>,
    pub list_requests: bool,
    pub known_requests: BTreeSet<Hash>,
}

impl Vault {
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
        let Some(profile) = self.profile.as_ref() else {
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

    pub(super) fn has_delivery(&self, session: Hash, slot: Slot) -> bool {
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

    pub(super) fn queue_pairing(
        &mut self,
        session: Hash,
        slot: Slot,
        bytes: Vec<u8>,
    ) -> Result<()> {
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

    pub(super) fn save_pairing(&mut self) -> Result<()> {
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
