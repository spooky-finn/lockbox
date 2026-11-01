//! Durable local issuance policy and exact retry artifacts. Shared authority is
//! established by signed controls, never by this installation-local ledger.

use crate::core::sync::{Admission, AdmissionReceipt, Hash, Invitation, canonical};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationStatus {
    Pending,
    Cancelled,
    Expired,
    Admitting,
    Admitted,
}

pub struct IssuedInvitationInfo {
    pub id: Hash,
    pub user: Hash,
    pub device: Hash,
    pub role: crate::core::sync::Role,
    pub status: InvitationStatus,
}

pub struct AcceptedInvitationInfo {
    pub id: Hash,
    pub role: crate::core::sync::Role,
    pub joined: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharingState {
    pub(crate) issued: BTreeMap<Hash, IssuedInvitation>,
    pub(crate) accepted: BTreeMap<Hash, AcceptedInvitation>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssuedInvitation {
    pub(crate) invitation: Invitation,
    pub(crate) cancelled: bool,
    pub(crate) admission: Option<Admission>,
    pub(crate) receipt: Option<AdmissionReceipt>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedInvitation {
    pub(crate) invitation: Invitation,
    pub(crate) admission: Admission,
    pub(crate) joined: Option<Hash>,
}

impl SharingState {
    pub(crate) const MAX_RECORDS: usize = 128;
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    pub(crate) fn is_empty(&self) -> bool {
        self.issued.is_empty() && self.accepted.is_empty()
    }

    pub(crate) fn validate(&self) -> bool {
        self.issued.len() + self.accepted.len() <= Self::MAX_RECORDS
            && self.issued.iter().all(|(id, record)| {
                *id == record.invitation.id()
                    && record.admission.as_ref().is_none_or(|a| a.id() == *id)
                    && record
                        .receipt
                        .as_ref()
                        .is_none_or(|r| r.id() == *id && record.admission.is_some())
                    && !(record.cancelled && record.admission.is_some())
            })
            && self
                .accepted
                .iter()
                .all(|(id, record)| *id == record.invitation.id() && record.admission.id() == *id)
            && canonical(self).is_ok_and(|bytes| bytes.len() <= Self::MAX_BYTES)
    }
}

impl IssuedInvitation {
    pub(crate) fn status(&self, now: u64) -> InvitationStatus {
        if self.receipt.is_some() {
            InvitationStatus::Admitted
        } else if self.admission.is_some() {
            InvitationStatus::Admitting
        } else if self.cancelled {
            InvitationStatus::Cancelled
        } else if self.invitation.expires_at().is_some_and(|end| now >= end) {
            InvitationStatus::Expired
        } else {
            InvitationStatus::Pending
        }
    }
}
