//! Wire schemas and immutable object dependencies. No authorization or I/O.

use crate::sync::protocol::{
    Admission, Blob, Certificate, Counter, Hash, Object, Role, Signed,
    machine::state::SpaceDescriptor,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub certificate: Signed<Certificate>,
    pub role: Role,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Epoch {
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
    // Keep the optional detached statement out of every control's inline storage.
    pub(super) approval: Option<Box<Signed<ControlApproval>>>,
}

impl Control {
    pub(super) fn is_recovery(&self) -> bool {
        self.transition == Some(ControlTransition::Recovery)
    }

    pub(super) fn frontier_limit(&self) -> usize {
        if self.is_recovery() {
            512
        } else {
            crate::sync::protocol::MAX_PARENTS
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlTransition {
    Transfer,
    Recovery,
}

/// Detached approval binds every semantic field of a prepared control and the
/// complete descriptor. The final author also signs this approval's exact bytes.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlApproval {
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
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = Hash> + '_ {
        let (first, parents, last, limit) = match self {
            Self::Control(control) => (
                control.body.epoch.previous,
                control.body.closed_frontier.as_slice(),
                None,
                control.body.frontier_limit(),
            ),
            Self::Change(change) => (
                Some(change.body.header.membership),
                change.body.header.parents.as_slice(),
                change.body.header.previous,
                crate::sync::protocol::MAX_PARENTS,
            ),
        };
        first
            .into_iter()
            .chain(parents.iter().take(limit).copied())
            .chain(last)
    }
}
