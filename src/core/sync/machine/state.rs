//! Persisted ciphertext journal, pinned trust anchors, and replay results.

use super::super::{Blob, Error, Hash, Projection, Role, Transaction};
use super::protocol::{Change, Control};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Exact encrypted body reserved before signing. Persist this in the encrypted
/// profile before calling complete_intent; retries must never prepare a new body.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PreparedChange {
    Change(Change),
    Control(Control),
}

/// Device-local journal. Object bytes are ciphertext or public signed metadata;
/// this type contains no signing secret, content key, or decrypted projection.
///
/// Hosts serialize it inside an encrypted local profile. Retain exact object and
/// intent bytes across retries; provider listings must never replace this journal.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncState {
    /// Cryptographic scope for every signed and encrypted object.
    pub space: Hash,
    /// Independent provider routing value; possession grants no authority.
    pub namespace: Hash,
    /// Pinned complete-object hash, established outside the provider.
    pub genesis: Hash,
    /// Owner trust anchor, compared through an authenticated channel on join.
    pub owner_root: [u8; 32],
    /// Immutable candidate objects; only replay establishes their authority.
    pub objects: BTreeMap<Hash, Blob>,
    /// Locally committed objects awaiting provider acknowledgement.
    pub outbox: BTreeSet<Hash>,
    /// Exact unsigned encrypted body that must reach durable storage before signing.
    pub intent: Option<PreparedChange>,
    /// Persisted integrity fault. Provider availability must not clear it.
    pub fault: Option<Error>,
    /// Last successful exchange time for display, never an ordering or trust input.
    pub last_exchange: Option<u64>,
}

/// Public join metadata. Authenticate the complete value independently before use;
/// it contains no provider credentials, private identity, or epoch key.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceDescriptor {
    pub space: Hash,
    pub namespace: Hash,
    pub genesis: Hash,
    pub owner_root: [u8; 32],
}

impl SyncState {
    /// Export public routing and identity anchors without granting membership.
    pub fn descriptor(&self) -> SpaceDescriptor {
        SpaceDescriptor {
            space: self.space,
            namespace: self.namespace,
            genesis: self.genesis,
            owner_root: self.owner_root,
        }
    }
}

/// Verified reference view and its readiness/diagnostic facts.
///
/// A non-ready view must not replace the last readable local snapshot or authorize
/// publication. `rejected` candidates are not accepted transactions; `pending`
/// dependencies do not establish corruption. A ready view can still contain
/// concurrent versions that require explicit resolution.
pub struct Replay {
    pub membership: Hash,
    pub epoch: u64,
    pub projection: Projection,
    pub frontier: BTreeSet<Hash>,
    /// Accepted change hashes, including finalized history the device can replay.
    pub accepted: BTreeSet<Hash>,
    pub pending: BTreeSet<Hash>,
    pub rejected: BTreeMap<Hash, Error>,
    /// Closed-epoch drafts outside the owner's committed frontier ancestry.
    pub excluded: BTreeSet<Hash>,
    pub role: Option<Role>,
    pub ready: bool,
    /// False for a new member bootstrapping history it cannot decrypt.
    pub history_verified: bool,
}

/// An old signed edit available for review; it grants no current write authority.
pub struct ExcludedEdit {
    pub hash: Hash,
    pub author: Hash,
    pub transaction: Transaction,
}
