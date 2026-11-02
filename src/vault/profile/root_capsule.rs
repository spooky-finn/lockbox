//! Password-encrypted owner root, independent of device and sync state.

use crate::{
    sync::identity::Key,
    vault::{KdfParams, Result, VaultError},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(super) struct RootCapsule {
    #[zeroize(skip)]
    pub(super) version: u16,
    #[zeroize(skip)]
    pub(super) kdf: KdfParams,
    salt: [u8; 16],
    nonce: [u8; 24],
    pub(super) ciphertext: Vec<u8>,
}

impl RootCapsule {
    fn key(&self, password: &SecretString) -> Result<Zeroizing<Key>> {
        let stretched = crate::vault::format::stretch(password, &self.salt, self.kdf)?;
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, stretched.as_ref())
            .expand(b"lockbox.root-capsule.v1\0", key.as_mut())
            .expect("fixed HKDF output size");
        Ok(key)
    }

    fn aad(&self, public_key: &Key) -> Vec<u8> {
        let mut aad = b"lockbox.root-capsule.v1\0".to_vec();
        aad.extend_from_slice(&self.version.to_le_bytes());

        for n in [self.kdf.memory_kib, self.kdf.passes, self.kdf.lanes] {
            aad.extend_from_slice(&n.to_le_bytes());
        }
        aad.extend_from_slice(&self.salt);
        aad.extend_from_slice(public_key);
        aad
    }

    pub(super) fn seal(root: &SigningKey, password: &SecretString, kdf: KdfParams) -> Result<Self> {
        let mut capsule = Self {
            version: 1,
            kdf: kdf.validate()?,
            salt: [0; 16],
            nonce: [0; 24],
            ciphertext: Vec::new(),
        };
        getrandom::fill(&mut capsule.salt).map_err(|_| VaultError::Random)?;
        getrandom::fill(&mut capsule.nonce).map_err(|_| VaultError::Random)?;
        let key = capsule.key(password)?;
        capsule.ciphertext = XChaCha20Poly1305::new(key.as_ref().into())
            .encrypt(
                XNonce::from_slice(&capsule.nonce),
                Payload {
                    msg: root.as_bytes(),
                    aad: &capsule.aad(&root.verifying_key().to_bytes()),
                },
            )
            .map_err(|_| VaultError::Corrupted)?;
        Ok(capsule)
    }

    pub(super) fn open(&self, password: &SecretString, public_key: &Key) -> Result<SigningKey> {
        if self.version != 1 {
            return Err(VaultError::Corrupted);
        }

        let key = self.key(password)?;
        let plaintext = Zeroizing::new(
            XChaCha20Poly1305::new(key.as_ref().into())
                .decrypt(
                    XNonce::from_slice(&self.nonce),
                    Payload {
                        msg: &self.ciphertext,
                        aad: &self.aad(public_key),
                    },
                )
                .map_err(|_| VaultError::Corrupted)?,
        );
        let seed: Zeroizing<Key> = Zeroizing::new(
            plaintext
                .as_slice()
                .try_into()
                .map_err(|_| VaultError::Corrupted)?,
        );
        let root = SigningKey::from_bytes(&seed);

        if root.verifying_key().to_bytes() != *public_key {
            return Err(VaultError::Corrupted);
        }

        Ok(root)
    }
}
