//! BIP39 English recovery words; Ed25519 roots use the SLIP-0010 master key (m).
//! The BIP39 passphrase is always empty, so the words alone recover the keys.

use crate::sync::identity::Key;
use bip39::{Language, Mnemonic};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use secrecy::SecretString;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not obtain random data from the operating system")]
    Random,
    #[error("Enter a valid 12-word English recovery phrase")]
    InvalidPhrase,
}

/// Secret recovery material. Deliberately has no Debug or serialization support.
pub struct RecoveryPhrase(Mnemonic);

impl RecoveryPhrase {
    pub fn generate() -> Result<Self, Error> {
        let mut entropy = Zeroizing::new([0u8; 16]);
        getrandom::fill(entropy.as_mut()).map_err(|_| Error::Random)?;
        Ok(Self(
            Mnemonic::from_entropy(entropy.as_ref()).expect("128-bit entropy"),
        ))
    }

    pub fn parse(words: &str) -> Result<Self, Error> {
        // English words need no Unicode normalization. Parsing checks the checksum
        // and accepts whitespace-separated words without copying the input.
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|_| Error::InvalidPhrase)?;
        if mnemonic.word_count() != 12 {
            return Err(Error::InvalidPhrase);
        }
        Ok(Self(mnemonic))
    }

    pub fn words(&self) -> SecretString {
        SecretString::from(self.0.to_string())
    }

    pub(crate) fn signing_key(&self) -> SigningKey {
        Self::root_from_seed(self.seed().as_ref())
    }

    fn seed(&self) -> Zeroizing<[u8; 64]> {
        Zeroizing::new(self.0.to_seed_normalized(""))
    }

    fn root_from_seed(seed: &[u8]) -> SigningKey {
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(b"ed25519 seed")
            .expect("HMAC accepts any key length");
        mac.update(seed);
        let output: Zeroizing<[u8; 64]> = Zeroizing::new(mac.finalize().into_bytes().into());
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&output[..32]);
        SigningKey::from_bytes(&key)
    }

    /// Independent key domain and per-file salt for the recovery key slot.
    pub(crate) fn wrapping_key(&self, salt: &[u8]) -> Zeroizing<Key> {
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(salt), self.seed().as_ref())
            .expand(b"lockbox.recovery-wrap.v1\0", key.as_mut())
            .expect("fixed HKDF output size");
        key
    }
}

#[cfg(test)]
#[path = "tests/mnemonic.rs"]
mod tests;
