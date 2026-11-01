//! Wire schemas and immutable object dependencies. No authorization or I/O.

use super::super::{Admission, Blob, Certificate, Counter, Hash, Object, Role, Signed};
use serde::{Deserialize, Serialize};
use super::state::SpaceDescriptor;

pub(super) use super::profile::PROFILE;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub certificate: Signed<Certificate>,
    pub role: Role,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Epoch {
    pub(super) profile: String,
    pub(super) space: Hash,
    pub(super) number: Counter,
    pub(super) context: Hash,
    pub(super) previous: Option<Hash>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    pub(super) device: Hash,
    pub(super) enc: Blob,
    pub(super) ciphertext: Blob,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub(super) epoch: Epoch,
    pub(super) owner: Hash,
    pub(super) controller: Hash,
    pub(super) members: Vec<Member>,
    pub(super) closed_frontier: Vec<Hash>,
    pub(super) grants: Vec<Grant>,
    pub(super) checkpoint_nonce: Blob,
    pub(super) checkpoint: Blob,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) admissions: Vec<Admission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) transition: Option<ControlTransition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) approval: Option<Signed<ControlApproval>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlTransition { Transfer, Recovery }

/// Detached approval binds every semantic field of a prepared control and the
/// complete descriptor. The final author also signs this approval's exact bytes.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlApproval {
    pub(super) profile: String,
    pub(super) descriptor: SpaceDescriptor,
    pub(super) statement: Hash,
    pub(super) author: Hash,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlTransfer {
    pub(super) descriptor: SpaceDescriptor,
    pub(super) control: Control,
    pub(super) signature: Blob,
    pub(super) objects: Vec<Blob>,
}

/// Independently authenticated current content boundary. Frontier hashes commit
/// to encrypted events; no password-derived plaintext digest is made public.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryAnchor {
    pub descriptor: SpaceDescriptor,
    pub control: Hash,
    pub frontier: Vec<Hash>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeHeader {
    pub(super) profile: String,
    pub(super) space: Hash,
    pub(super) membership: Hash,
    pub(super) epoch: Counter,
    pub(super) author: Hash,
    pub(super) sequence: Counter,
    pub(super) previous: Option<Hash>,
    pub(super) parents: Vec<Hash>,
    pub(super) nonce: Blob,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub(super) header: ChangeHeader,
    pub(super) ciphertext: Blob,
}

impl Object {
    /// Bounded availability hints. Replay checks the complete signed lists and
    /// rejects an over-limit object; transport never expands unbounded references.
    pub(crate) fn dependencies(&self) -> Vec<Hash> {
        match self {
            Self::Control(control) => control
                .body
                .epoch
                .previous
                .into_iter()
                .chain(
                    control
                        .body
                        .closed_frontier
                        .iter()
                        .take(if control.body.transition == Some(ControlTransition::Recovery) { 512 } else { super::super::MAX_PARENTS })
                        .copied(),
                )
                .collect(),
            Self::Change(change) => std::iter::once(change.body.header.membership)
                .chain(
                    change
                        .body
                        .header
                        .parents
                        .iter()
                        .take(super::super::MAX_PARENTS)
                        .copied(),
                )
                .chain(change.body.header.previous)
                .collect(),
        }
    }
}
