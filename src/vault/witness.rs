//! Independent profile witnesses and exclusive, installation-wide signer leases.
//! Provider credentials, record plaintext and private keys never enter this port.

use crate::{
    sync::protocol::Hash,
    vault::anchor::{Error, Result, Witness},
};
use std::sync::Arc;

mod directory;
#[cfg(target_os = "macos")]
mod keychain;
mod memory;

pub use directory::DirectoryWitness;
pub use memory::MemoryWitness;

/// Address records by device identity, independently of profile path and salt.
pub trait WitnessStore: Send + Sync {
    /// Hold an exclusive installation-wide lease until the returned object drops.
    /// Copies at different paths must contend for the same signer lease.
    fn acquire(&self, device: Hash) -> Result<Box<dyn WitnessLease>>;
}

pub trait WitnessLease: Send {
    /// Only an absent item is `None`; denial, corruption and I/O are errors.
    fn load(&self) -> Result<Option<Witness>>;
    /// Compare and durably replace while the exclusive signer lease is held.
    /// A lost acknowledgement may return an error after the exact value persisted.
    fn replace(&mut self, expected: Option<Witness>, next: Witness) -> Result<()>;
}

pub(crate) fn system() -> Arc<dyn WitnessStore> {
    #[cfg(test)]
    {
        static STORE: std::sync::OnceLock<Arc<MemoryWitness>> = std::sync::OnceLock::new();
        STORE
            .get_or_init(|| Arc::new(MemoryWitness::default()))
            .clone()
    }
    #[cfg(not(test))]
    {
        let Some(root) = dirs::data_local_dir().map(|path| path.join("lockbox/signer-witnesses"))
        else {
            return Arc::new(Unavailable);
        };
        #[cfg(target_os = "macos")]
        {
            Arc::new(keychain::KeychainWitness::new(root))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Arc::new(DirectoryWitness::new(root))
        }
    }
}

#[cfg(not(test))]
struct Unavailable;

#[cfg(not(test))]
impl WitnessStore for Unavailable {
    fn acquire(&self, _: Hash) -> Result<Box<dyn WitnessLease>> {
        Err(Error::Unavailable)
    }
}
