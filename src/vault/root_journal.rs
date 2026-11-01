//! A separately addressed owner-root signer journal shares the durable CAS
//! adapter with device witnesses. Its record maps target profile, prepared claim,
//! epoch and space digest to the adapter's four metadata fields. It never enters
//! profile-generation reconciliation and contains no signing secret.

use crate::{
    sync::protocol::{Error, Hash, Machine, canonical},
    vault::{
        Result, Vault,
        anchor::{self, Witness},
        witness::WitnessLease,
    },
};

pub(super) fn reserve(vault: &Vault) -> Result<Box<dyn WitnessLease>> {
    let state = vault.sync_state().ok_or(Error::UnauthorizedDevice)?;
    let (claim, epoch) = Machine::root_recovery_claim(state)?;
    let scope = Hash::digest(
        b"lockbox.root-recovery-journal.v1\0",
        &canonical(&state.descriptor())?,
    );
    let expected = Witness {
        profile: Hash::digest(b"lockbox.profile-id.v1\0", &vault.salt),
        binding: claim,
        generation: epoch,
        digest: Some(scope),
    };
    let mut lease = vault.witnesses.acquire(scope)?;
    let previous = lease.load()?;
    if let Some(saved) = previous {
        if saved == expected {
            return Ok(lease);
        }
        if saved.digest != Some(scope) {
            return Err(anchor::Error::Rollback.into());
        }
        if saved.generation >= epoch || !Machine::has_root_recovery_claim(state, saved.binding)? {
            return Err(Error::RecoveryReserved.into());
        }
    }
    lease.replace(previous, expected)?;
    Ok(lease)
}
