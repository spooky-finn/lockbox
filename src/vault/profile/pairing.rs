//! Durable link policy and exact encrypted deliveries, separate from membership authority.

use crate::sync::protocol::{
    Blob, Hash, Invitation, Role,
    pairing::{ClosedReason, Closure, Contact, LinkReference, Ticket},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingState {
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
    pub proof_verified: bool,
    pub request: Option<Contact>,
    pub offer: Option<Invitation>,
    pub consented: bool,
    pub joined: bool,
    pub closed: Option<ClosedReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum Slot {
    Ticket(Hash),
    Bootstrap,
    Request(Hash),
    Offer(Hash),
    Consent(Hash),
    Receipt(Hash),
    Joined(Hash),
    Rejected(Hash),
    Status,
}

impl Slot {
    pub fn key(self) -> String {
        match self {
            Self::Ticket(_) => "ticket".into(),
            Self::Bootstrap => "bootstrap".into(),
            Self::Status => "status".into(),
            Self::Request(id) => format!("requests/{id}"),
            Self::Offer(id) => format!("offers/{id}"),
            Self::Consent(id) => format!("consents/{id}"),
            Self::Receipt(id) => format!("receipts/{id}"),
            Self::Joined(id) => format!("joined/{id}"),
            Self::Rejected(id) => format!("rejected/{id}"),
        }
    }

    /// Ticket lookup precedes discovery of the random pairing session ID.
    pub fn address(self, session: Hash) -> Hash {
        match self {
            Self::Ticket(hash) => hash,
            _ => session,
        }
    }

    pub fn budget(self) -> usize {
        match self {
            Self::Ticket(_) => crate::sync::protocol::pairing::MAX_TICKET_BYTES,
            Self::Bootstrap => crate::sync::protocol::MAX_INVITATION_BYTES,
            Self::Offer(_) | Self::Receipt(_) => crate::sync::protocol::pairing::MAX_MESSAGE_BYTES,
            _ => crate::sync::protocol::pairing::MAX_CONTACT_BYTES,
        }
    }
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
        self.issued.is_empty()
            && self.joining.is_empty()
            && self.outgoing.is_empty()
            && self.resolving.is_none()
    }

    pub fn uses_short_links(&self) -> bool {
        self.resolving.is_some()
            || self
                .outgoing
                .iter()
                .any(|delivery| matches!(delivery.slot, Slot::Ticket(_)))
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
            && self.outgoing.len() <= 256
            && self.issued.iter().all(|(id, link)| {
                *id == link.ticket.body.id
                    && link.requests.len() <= Self::MAX_REQUESTS
                    && link.ticket.verify_ticket().is_ok()
                    && link.requests.iter().all(|(hash, request)| {
                        request.hash().ok() == Some(*hash)
                            && request.verify_contact(&link.ticket).is_ok()
                    })
                    && link.approval.as_ref().is_none_or(|approval| {
                        link.requests.contains_key(&approval.request)
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
                    && link.ticket.verify_ticket().is_ok()
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
