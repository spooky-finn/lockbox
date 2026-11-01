//! Authenticated key slots for v8 profiles. Neither slot contains plaintext keys.

use crate::{
    access::mnemonic::RecoveryPhrase,
    sync::identity::Key,
    vault::{Result, VaultError, format::cipher},
};
use chacha20poly1305::{
    XNonce,
    aead::{Aead, Payload},
};
use zeroize::Zeroizing;

pub(super) const SLOT_LEN: usize = 24 + 32 + 16;
pub(super) const SLOTS_LEN: usize = 2 * SLOT_LEN;

#[derive(Clone)]
pub(super) struct RecoverySlots {
    pub(super) password: [u8; SLOT_LEN],
    pub(super) phrase: [u8; SLOT_LEN],
}

impl RecoverySlots {
    pub(super) fn create(
        key: &Key,
        password_key: &Key,
        phrase: &RecoveryPhrase,
        salt: &[u8],
    ) -> Result<Self> {
        Ok(Self {
            password: seal(key, password_key, salt, b"password")?,
            phrase: seal(key, &phrase.wrapping_key(salt), salt, b"phrase")?,
        })
    }

    pub(super) fn password_key(&self, key: &Key, salt: &[u8]) -> Result<Zeroizing<Key>> {
        open(&self.password, key, salt, b"password").ok_or(VaultError::Corrupted)
    }

    pub(super) fn phrase_key(
        &self,
        phrase: &RecoveryPhrase,
        salt: &[u8],
    ) -> Result<Zeroizing<Key>> {
        open(&self.phrase, &phrase.wrapping_key(salt), salt, b"phrase")
            .ok_or(VaultError::WrongRecoveryPhrase)
    }

    pub(super) fn reset_password(
        &mut self,
        key: &Key,
        password_key: &Key,
        salt: &[u8],
    ) -> Result<()> {
        self.password = seal(key, password_key, salt, b"password")?;
        Ok(())
    }
}

fn aad(salt: &[u8], kind: &[u8]) -> Vec<u8> {
    [b"lockbox.key-slot.v1\0".as_slice(), salt, kind].concat()
}

fn seal(key: &Key, wrapping_key: &Key, salt: &[u8], kind: &[u8]) -> Result<[u8; SLOT_LEN]> {
    let mut slot = [0u8; SLOT_LEN];
    getrandom::fill(&mut slot[..24]).map_err(|_| VaultError::Random)?;
    let ciphertext = cipher(wrapping_key)
        .encrypt(
            XNonce::from_slice(&slot[..24]),
            Payload {
                msg: key,
                aad: &aad(salt, kind),
            },
        )
        .map_err(|_| VaultError::Corrupted)?;
    slot[24..].copy_from_slice(&ciphertext);
    Ok(slot)
}

fn open(slot: &[u8; SLOT_LEN], key: &Key, salt: &[u8], kind: &[u8]) -> Option<Zeroizing<Key>> {
    let plaintext = Zeroizing::new(
        cipher(key)
            .decrypt(
                XNonce::from_slice(&slot[..24]),
                Payload {
                    msg: &slot[24..],
                    aad: &aad(salt, kind),
                },
            )
            .ok()?,
    );
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&plaintext);
    Some(key)
}
