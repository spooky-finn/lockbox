//! Deterministic, provider-independent shared-space state machine.
//!
//! `LBX-SYNC-0` is an experimental, deliberately separate wire profile from the
//! proposal in docs/lockbox-protocol.md. Only canonical objects enter replay.
//!
//! # Host durability contract
//!
//! [`Machine`] separates unsigned command preparation from signing. The host owns
//! storage and publication; it must commit each boundary in this order:
//!
//! ```no_run
//! use lockbox::core::sync::{DeviceSecret, Hash, Machine, SyncState, Transaction};
//!
//! fn save_edit(
//!     state: &mut SyncState,
//!     device: &DeviceSecret,
//!     edit: Transaction,
//!     mut persist: impl FnMut(&SyncState) -> std::io::Result<()>,
//! ) -> Result<Hash, Box<dyn std::error::Error>> {
//!     state.intent = Some(Machine::prepare(state, device, edit)?);
//!     persist(state)?; // reserve the exact encrypted signing body
//!     let hash = Machine::complete_intent(state, device)?;
//!     persist(state)?; // commit event, sequence and outbox before upload
//!     Ok(hash)
//! }
//! ```
//!
//! An error after either persistence call must retain the same intent/event for
//! recovery. [`crate::infra::vault::Vault`] implements this contract atomically
//! inside its encrypted profile and reconciles an independent installation witness
//! before signing or exchange. Other production hosts must supply equivalent
//! rollback protection in their persistence boundary. Core replay alone cannot
//! detect a restored profile when all independent evidence is absent.
//! Storage adapters receive only object bytes and
//! never construct verified transactions or choose conflict winners.

mod crypto;
mod machine;
mod model;
mod wire;

pub use crypto::{Certificate, DeviceSecret, Signed};
pub use machine::{ExcludedEdit, Machine, PreparedChange, Replay, SpaceDescriptor, SyncState};
pub use model::{
    Action, Entity, EntityKind, Mutation, Projection, Record, Revision, Role, Transaction, Value,
    Version,
};
pub use wire::{Blob, Counter, Hash, Object, canonical, decode};

pub const MAX_OBJECT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRANSACTION_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OBJECTS: usize = 4096;
pub const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PARENTS: usize = 64;
pub const MAX_MUTATIONS: usize = 256;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error,
)]
pub enum Error {
    #[error("invalid or noncanonical encoding")]
    InvalidEncoding,
    #[error("unsupported synchronization profile")]
    UnsupportedProtocol,
    #[error("object hash mismatch")]
    HashMismatch,
    #[error("invalid signature or certificate")]
    InvalidSignature,
    #[error("device is not admitted")]
    UnauthorizedDevice,
    #[error("role does not allow this operation")]
    InsufficientRole,
    #[error("history dependencies are missing")]
    MissingDependency,
    #[error("epoch key is unavailable")]
    KeyUnavailable,
    #[error("encrypted data authentication failed")]
    AuthenticationFailed,
    #[error("conflicting membership successors; publication stopped")]
    ControlFork,
    #[error("device signed conflicting sequence slots; publication stopped")]
    DeviceFork,
    #[error("checkpoint does not match known history")]
    CheckpointMismatch,
    #[error("invalid causal history or transaction")]
    InvalidTransaction,
    #[error("synchronization resource budget exceeded")]
    ResourceBudgetExceeded,
    #[error("causal frontier must be folded before preparing this operation")]
    FrontierTooWide,
    #[error("a durable signing intent must be completed first")]
    PendingIntent,
    #[error("operating system randomness unavailable")]
    Random,
}

impl Error {
    /// Faults in authenticated history freeze local writes and publication.
    /// Rejected candidates and missing dependencies do not revoke membership.
    pub fn is_history_fault(self) -> bool {
        matches!(
            self,
            Self::ControlFork
                | Self::DeviceFork
                | Self::CheckpointMismatch
                | Self::AuthenticationFailed
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
