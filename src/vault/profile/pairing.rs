//! Durable link policy and exact encrypted deliveries, separate from membership authority.

use crate::sync::protocol::{
    Blob, Hash, Invitation, Role,
    pairing::{ClosedReason, Closure, Contact, LinkReference, ManagedLink, Slot, Ticket},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_grants: BTreeMap<Hash, String>,
    pub issued: BTreeMap<Hash, IssuedLink>,
    pub joining: BTreeMap<Hash, JoiningLink>,
    pub outgoing: Vec<Delivery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolving: Option<ResolvingLink>,
    #[serde(skip)]
    pub needs_save: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolvingLink {
    pub reference: LinkReference,
    pub code: Hash,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssuedLink {
    pub ticket: Ticket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<ManagedLink>,
    pub requests: BTreeMap<Hash, Contact>,
    pub rejected: BTreeSet<Hash>,
    pub approval: Option<Approval>,
    pub closed: Option<Closure>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Approval {
    pub request: Hash,
    pub role: Role,
    pub invitation: Option<Hash>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoiningLink {
    pub ticket: Ticket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<ManagedLink>,
    /// Set only by explicit Join after opening a signed managed chat link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_join: Option<Hash>,
    pub proof_verified: bool,
    pub request: Option<Contact>,
    pub offer: Option<Invitation>,
    pub consented: bool,
    pub joined: bool,
    pub closed: Option<ClosedReason>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Delivery {
    pub session: Hash,
    pub slot: Slot,
    pub bytes: Blob,
    pub published: bool,
}

impl PairingState {
    pub const MAX_LINKS: usize = 16;
    pub const MAX_REQUESTS: usize = 32;

    pub fn is_empty(&self) -> bool {
        self.device_grants.is_empty()
            && self.issuer.is_none()
            && self.issued.is_empty()
            && self.joining.is_empty()
            && self.outgoing.is_empty()
            && self.resolving.is_none()
    }

    pub fn uses_managed_links(&self) -> bool {
        self.issuer.is_some()
            || !self.device_grants.is_empty()
            || self.joining.values().any(|l| l.managed.is_some())
            || self.issued.values().any(|l| l.managed.is_some())
    }

    pub fn uses_chat_links(&self) -> bool {
        self.issued
            .values()
            .any(|l| l.ticket.body.chat_role.is_some())
            || self
                .joining
                .values()
                .any(|l| l.ticket.body.chat_role.is_some())
    }

    pub fn uses_short_links(&self) -> bool {
        self.resolving.is_some()
            || self
                .outgoing
                .iter()
                .any(|delivery| matches!(delivery.slot, Slot::Ticket(_)))
    }

    fn valid_mailbox(ticket: &Ticket, managed: Option<&ManagedLink>) -> bool {
        match managed {
            Some(link) => {
                link.verify_mailbox().is_ok() && link.body.ticket.hash().ok() == ticket.hash().ok()
            }
            None => ticket.body.chat_role.is_none() && ticket.verify_ticket().is_ok(),
        }
    }

    pub fn validate(&self) -> bool {
        let mut deliveries = BTreeSet::new();
        self.issued.len() + self.joining.len() + usize::from(self.resolving.is_some())
            <= Self::MAX_LINKS
            && self.resolving.as_ref().is_none_or(|pending| {
                pending.reference.validate()
                    && !self
                        .joining
                        .values()
                        .any(|link| !link.joined && link.closed.is_none())
            })
            && (self.device_grants.is_empty() || self.issuer.is_some())
            && self.outgoing.len() <= 256
            && self.issued.iter().all(|(id, link)| {
                *id == link.ticket.body.id
                    && link.requests.len() <= Self::MAX_REQUESTS
                    && Self::valid_mailbox(&link.ticket, link.managed.as_ref())
                    && link.requests.iter().all(|(hash, request)| {
                        request.hash().ok() == Some(*hash)
                            && request.verify_contact(&link.ticket).is_ok()
                    })
                    && link.approval.as_ref().is_none_or(|approval| {
                        link.requests.contains_key(&approval.request)
                            && link
                                .ticket
                                .body
                                .chat_role
                                .is_none_or(|role| approval.role == role)
                            && !link.rejected.contains(&approval.request)
                            && matches!(approval.role, Role::Reader | Role::Editor | Role::Owner)
                    })
                    && link
                        .closed
                        .as_ref()
                        .is_none_or(|closed| closed.verify_closure(&link.ticket).is_ok())
            })
            && self.joining.iter().all(|(id, link)| {
                *id == link.ticket.body.id
                    && Self::valid_mailbox(&link.ticket, link.managed.as_ref())
                    && link.chat_join
                        == link
                            .ticket
                            .body
                            .chat_role
                            .and_then(|_| link.ticket.hash().ok())
                    && link
                        .request
                        .as_ref()
                        .is_none_or(|request| request.verify_contact(&link.ticket).is_ok())
                    && (!link.consented || link.offer.is_some())
                    && (!link.joined || link.consented)
            })
            && self.outgoing.iter().all(|delivery| {
                (self.issued.contains_key(&delivery.session)
                    || self.joining.contains_key(&delivery.session))
                    && delivery.bytes.0.len() <= delivery.slot.budget()
                    && deliveries.insert((delivery.session, delivery.slot))
                    && match delivery.slot {
                        Slot::Ticket(hash) => {
                            self.issued.get(&delivery.session).is_some_and(|link| {
                                link.ticket.hash().ok() == Some(hash)
                                    && crate::sync::protocol::canonical(&link.ticket)
                                        .ok()
                                        .as_deref()
                                        == Some(delivery.bytes.0.as_slice())
                            })
                        }
                        _ => true,
                    }
            })
            && crate::serialization::canonical_size(self).is_ok_and(|size| size <= 64 * 1024 * 1024)
    }
}

impl Drop for PairingState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.issuer.zeroize();
        for grant in self.device_grants.values_mut() {
            grant.zeroize();
        }
    }
}
impl IssuedLink {
    pub fn public_link(&self) -> crate::sync::protocol::Result<String> {
        match &self.managed {
            Some(managed) => managed.link(),
            None => Ok(self.ticket.reference()?.link()),
        }
    }
}
