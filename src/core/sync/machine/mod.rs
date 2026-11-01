//! Protocol facade with separate command preparation, verification, and schemas.
//!
//! Public methods are implemented by responsibility in `commands` and `replay`.
//! This facade never owns a provider, filesystem handle, clock, or UI state.

mod authority;
mod commands;
mod history;
mod invitations;
mod membership;
mod parents;
mod profile;
mod protocol;
mod relay;
mod replay;
mod state;

pub use protocol::{
    Change, Control, ControlApproval, ControlTransfer, ControlTransition, RecoveryAnchor,
};
pub use state::{ExcludedEdit, PreparedChange, Replay, SpaceDescriptor, SyncState};

/// Stateless entry point for deterministic replay and encrypted command preparation.
///
/// Hosts must durably save [`SyncState::intent`] before calling
/// [`Machine::complete_intent`], then commit the resulting state before uploading
/// any object. [`crate::infra::vault::Vault`] provides that persistence adapter.
pub struct Machine;

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
