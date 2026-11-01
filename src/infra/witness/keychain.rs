//! OS credential-store witness, separate from Touch ID's optional unlock key.

use super::{Error, Hash, Result, Witness, WitnessLease, WitnessStore, directory};
use security_framework::passwords::{get_generic_password, set_generic_password};
use std::path::PathBuf;

const SERVICE: &str = "lockbox.profile-witness.v1";

pub(super) struct KeychainWitness {
    root: PathBuf,
}

impl KeychainWitness {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl WitnessStore for KeychainWitness {
    fn acquire(&self, device: Hash) -> Result<Box<dyn WitnessLease>> {
        Ok(Box::new(Lease {
            account: device.to_string(),
            lock: directory::lock(&self.root, device)?,
        }))
    }
}

struct Lease {
    account: String,
    lock: directory::SignerLock,
}

impl WitnessLease for Lease {
    fn load(&self) -> Result<Option<Witness>> {
        self.lock.check()?;
        match get_generic_password(SERVICE, &self.account) {
            Ok(bytes) => directory::decode(&bytes).map(Some),
            Err(error) if error.code() == -25300 => Ok(None), // errSecItemNotFound only
            Err(_) => Err(Error::Unavailable),
        }
    }

    fn replace(&mut self, expected: Option<Witness>, next: Witness) -> Result<()> {
        if self.load()? != expected {
            return Err(Error::Rollback);
        }
        let bytes = serde_json::to_vec(&next).map_err(|_| Error::Unavailable)?;
        self.lock.check()?;
        set_generic_password(SERVICE, &self.account, &bytes).map_err(|_| Error::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "explicit OS probe: creates and removes one random witness item"]
    fn keychain_witness_round_trip() {
        use security_framework::passwords::delete_generic_password;
        struct Cleanup(Option<String>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(account) = &self.0 {
                    let _ = delete_generic_password(SERVICE, account);
                }
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).unwrap();
        let device = Hash(bytes);
        let store = KeychainWitness::new(directory.path().to_owned());
        let mut lease = store.acquire(device).unwrap();
        assert!(lease.load().unwrap().is_none());
        let mut cleanup = Cleanup(Some(device.to_string()));
        let first = Witness::reservation(Hash([1; 32]), Hash([2; 32]));
        lease.replace(None, first).unwrap();
        assert_eq!(lease.load().unwrap(), Some(first));
        let next = Witness {
            generation: 1,
            digest: Some(Hash([3; 32])),
            ..first
        };
        assert_eq!(lease.replace(None, next), Err(Error::Rollback));
        lease.replace(Some(first), next).unwrap();
        drop(lease);
        let lease = store.acquire(device).unwrap();
        assert_eq!(lease.load().unwrap(), Some(next));
        delete_generic_password(SERVICE, &device.to_string()).unwrap();
        assert!(lease.load().unwrap().is_none());
        cleanup.0 = None; // already removed; do not issue another delete
    }
}
