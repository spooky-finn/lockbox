//! Touch ID unlock with Secure Enclave-enforced biometric access control.
//!
//! macOS encrypts the key with a Secure Enclave key bound to the currently
//! enrolled fingerprints. Keychain stores only the encrypted envelope; neither
//! signing entitlements nor an unprotected vault-key fallback are needed.

use crate::sync::identity::Key;
use std::sync::Arc;

use zeroize::Zeroizing;

/// The master password is required again this long after it was last typed.
pub const PASSWORD_EVERY_SECS: u64 = 7 * 24 * 60 * 60;

const REASON: &str = "unlock lockbox";

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TouchIdError {
    #[error("Touch ID is unavailable on this computer")]
    Unavailable,
    #[error("Touch ID: {0}")]
    Failed(String),
    #[error("Touch ID is not enabled for this vault")]
    NoKey,
    #[error("7 days have passed since the last master password entry — enter it again")]
    Expired,
    #[error("system fingerprints have changed — enter the master password")]
    FingersChanged,
    #[error("keychain: {0}")]
    Keychain(String),
}

/// Touch ID and keychain access; tests substitute an in-memory fake.
pub trait Biometrics: Send + Sync {
    /// Opaque value that changes when fingers are added or removed;
    /// `None` when Touch ID is unavailable.
    fn domain_state(&self) -> Option<Vec<u8>>;
    /// Shows the Touch ID prompt; on success returns the current `domain_state`.
    fn authenticate(&self, reason: &str) -> Result<Vec<u8>, String>;
    /// Retrieval itself must enforce biometric authentication before releasing data.
    /// Only a missing item returns `Ok(None)`; cancellation and OS failures are errors.
    fn load(&self, account: &str, reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String>;
    fn store(&self, account: &str, data: &[u8]) -> Result<(), String>;
    fn delete(&self, account: &str);
}

/// Keychain item: key, when the password was last typed, and the finger set.
struct Record {
    key: Zeroizing<Key>,
    saved_at: u64,
    domain_state: Vec<u8>,
}

impl Record {
    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(40 + self.domain_state.len()));
        bytes.extend_from_slice(self.key.as_ref());
        bytes.extend_from_slice(&self.saved_at.to_le_bytes());
        bytes.extend_from_slice(&self.domain_state);
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            return None;
        }
        Some(Self {
            key: Zeroizing::new(bytes[..32].try_into().ok()?),
            saved_at: u64::from_le_bytes(bytes[32..40].try_into().ok()?),
            domain_state: bytes[40..].to_vec(),
        })
    }
}

pub fn available(biometrics: &dyn Biometrics) -> bool {
    biometrics.domain_state().is_some()
}

/// Asks for the finger, then returns the saved key. A key that expired or was
/// saved for a different finger set is deleted, so only the password opens.
pub fn unlock(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    now: u64,
) -> Result<Zeroizing<Key>, TouchIdError> {
    if !available(biometrics) {
        return Err(TouchIdError::Unavailable);
    }

    let record = biometrics
        .load(vault_id, REASON)
        .map_err(TouchIdError::Failed)?
        .and_then(|bytes| Record::decode(&bytes))
        .ok_or(TouchIdError::NoKey)?;
    let fingers = biometrics.domain_state().ok_or(TouchIdError::Unavailable)?;

    if record.domain_state != fingers {
        biometrics.delete(vault_id);
        return Err(TouchIdError::FingersChanged);
    }

    if now.saturating_sub(record.saved_at) >= PASSWORD_EVERY_SECS {
        biometrics.delete(vault_id);
        return Err(TouchIdError::Expired);
    }

    Ok(record.key)
}

/// Turns Touch ID on: the finger is checked before the key is stored.
pub fn enable(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &Key,
    now: u64,
) -> Result<(), TouchIdError> {
    if !available(biometrics) {
        return Err(TouchIdError::Unavailable);
    }

    let fingers = biometrics
        .authenticate(REASON)
        .map_err(TouchIdError::Failed)?;
    save(biometrics, vault_id, key, now, fingers)
}

/// After the master password was typed: stores a fresh key, restarting the 7 days.
pub fn refresh(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &Key,
    now: u64,
) -> Result<(), TouchIdError> {
    let fingers = biometrics.domain_state().ok_or(TouchIdError::Unavailable)?;
    save(biometrics, vault_id, key, now, fingers)
}

pub fn disable(biometrics: &dyn Biometrics, vault_id: &str) {
    biometrics.delete(vault_id);
}

fn save(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &Key,
    now: u64,
    domain_state: Vec<u8>,
) -> Result<(), TouchIdError> {
    let record = Record {
        key: Zeroizing::new(*key),
        saved_at: now,
        domain_state,
    };
    biometrics
        .store(vault_id, &record.encode())
        .map_err(TouchIdError::Keychain)
}

/// Touch ID of this computer, or a stand-in that reports it as unavailable.
pub fn system() -> Arc<dyn Biometrics> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(mac::MacBiometrics::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(Unsupported)
    }
}

#[cfg(not(target_os = "macos"))]
struct Unsupported;

#[cfg(not(target_os = "macos"))]
impl Biometrics for Unsupported {
    fn domain_state(&self) -> Option<Vec<u8>> {
        None
    }
    fn authenticate(&self, _reason: &str) -> Result<Vec<u8>, String> {
        Err("not supported".into())
    }
    fn load(&self, _account: &str, _reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        Err("not supported".into())
    }
    fn store(&self, _account: &str, _data: &[u8]) -> Result<(), String> {
        Err("not supported".into())
    }
    fn delete(&self, _account: &str) {}
}

#[cfg(target_os = "macos")]
#[path = "touchid/macos.rs"]
mod mac;

#[cfg(test)]
#[path = "tests/touchid.rs"]
pub(crate) mod tests;
