//! One encrypted user identity per installation, independent of space profiles.

use crate::{
    access::mnemonic::RecoveryPhrase,
    vault::{KdfParams, Result, Vault, VaultError, file, format, recovery::RecoverySlots},
};
use chacha20poly1305::{
    XNonce,
    aead::{Aead, Payload},
};
use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{fs, io, path::Path};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const MAGIC: &[u8; 6] = b"LBID\x01\x00";
const HEADER_LEN: usize = 6 + 12 + 16 + 24;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct Identity {
    root: [u8; 32],
    words: Option<String>,
}

impl Identity {
    pub(crate) fn from_phrase(phrase: &RecoveryPhrase) -> Self {
        Self {
            root: phrase.signing_key().to_bytes(),
            words: Some(phrase.words().expose_secret().to_owned()),
        }
    }

    pub(crate) fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.root)
    }

    pub(crate) fn phrase(&self) -> Result<Option<RecoveryPhrase>> {
        self.words
            .as_deref()
            .map(RecoveryPhrase::parse)
            .transpose()
            .map_err(Into::into)
    }

    pub(crate) fn words(&self) -> Option<&str> {
        self.words.as_deref()
    }

    /// Import an existing root without changing any space or device key.
    pub(crate) fn from_vault(vault: &Vault, password: &SecretString) -> Result<Self> {
        let root = vault
            .profile
            .as_ref()
            .ok_or(VaultError::RecoveryUnavailable)?
            .unlock_root(password)?;
        Ok(Self {
            root: root.to_bytes(),
            words: None,
        })
    }

    pub(crate) fn open(path: &Path, password: &SecretString) -> Result<Self> {
        Self::open_using(path, Some(password), None)
    }

    pub(crate) fn recover(
        path: &Path,
        phrase: &RecoveryPhrase,
        password: &SecretString,
    ) -> Result<Self> {
        let _lock = file::lock_profile(path)?;
        let mut identity = Self::open_using(path, None, Some(phrase))?;
        identity.words = Some(phrase.words().expose_secret().to_owned());
        let (header, ciphertext) = identity.encrypt(password, KdfParams::default())?;
        file::replace(path, &header, &ciphertext)?;
        file::sync_dir(path)?;
        Ok(identity)
    }

    fn open_using(
        path: &Path,
        password: Option<&SecretString>,
        phrase: Option<&RecoveryPhrase>,
    ) -> Result<Self> {
        let data = file::read_bounded(path, file::MAX_IDENTITY_BYTES)?;
        if data.len() < HEADER_LEN + super::recovery::SLOTS_LEN + 16 || &data[..6] != MAGIC {
            return Err(VaultError::NotAVault);
        }
        let number = |offset| u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        let kdf = KdfParams {
            memory_kib: number(6),
            passes: number(10),
            lanes: number(14),
        }
        .validate()?;
        let salt: [u8; 16] = data[18..34].try_into().unwrap();
        let header_len = HEADER_LEN + super::recovery::SLOTS_LEN;
        let slots = data
            .get(HEADER_LEN..header_len)
            .ok_or(VaultError::NotAVault)?;
        let slots = RecoverySlots {
            password: slots[..super::recovery::SLOT_LEN].try_into().unwrap(),
            phrase: slots[super::recovery::SLOT_LEN..].try_into().unwrap(),
        };
        let key = if let Some(phrase) = phrase {
            let root = Zeroizing::new(phrase.signing_key().to_bytes());
            slots.recovery_key(&root_recovery_key(&root, &salt), &salt)?
        } else {
            let password_key =
                identity_key(password.ok_or(VaultError::WrongPassword)?, &salt, kdf)?;
            slots
                .password_key(&password_key, &salt)
                .map_err(|_| VaultError::WrongPassword)?
        };
        let plaintext = Zeroizing::new(
            format::cipher(&key)
                .decrypt(
                    XNonce::from_slice(&data[34..58]),
                    Payload {
                        msg: &data[header_len..],
                        aad: &data[..header_len],
                    },
                )
                .map_err(|_| VaultError::WrongPassword)?,
        );
        let identity: Self =
            serde_json::from_slice(&plaintext).map_err(|_| VaultError::Corrupted)?;
        if phrase.is_some_and(|phrase| {
            phrase.signing_key().verifying_key() != identity.signing_key().verifying_key()
        }) {
            return Err(VaultError::WrongRecoveryPhrase);
        }
        if identity.phrase()?.is_some_and(|phrase| {
            phrase.signing_key().verifying_key() != identity.signing_key().verifying_key()
        }) {
            return Err(VaultError::Corrupted);
        }
        Ok(identity)
    }

    /// Never replace a saved identity, including concurrent first-time setup.
    pub(crate) fn save_new(
        &self,
        path: &Path,
        password: &SecretString,
        kdf: KdfParams,
    ) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Identity directory is unavailable"))?;
        let mut directory = fs::DirBuilder::new();
        directory.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(parent)?;
        let _lock = file::lock_profile(path)?;
        match fs::symlink_metadata(path) {
            Ok(_) => {
                let saved = Self::open(path, password)?;
                return if saved.root == self.root {
                    Ok(())
                } else {
                    Err(VaultError::IdentityMismatch)
                };
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let (header, ciphertext) = self.encrypt(password, kdf)?;
        file::create(path, &header, &ciphertext)?;
        file::sync_dir(path)?;
        Ok(())
    }

    fn encrypt(&self, password: &SecretString, kdf: KdfParams) -> Result<(Vec<u8>, Vec<u8>)> {
        let kdf = kdf.validate()?;
        let mut header = Vec::from(MAGIC.as_slice());
        for value in [kdf.memory_kib, kdf.passes, kdf.lanes] {
            header.extend_from_slice(&value.to_le_bytes());
        }
        let mut salt = [0; 16];
        let mut nonce = [0; 24];
        getrandom::fill(&mut salt).map_err(|_| VaultError::Random)?;
        getrandom::fill(&mut nonce).map_err(|_| VaultError::Random)?;
        header.extend_from_slice(&salt);
        header.extend_from_slice(&nonce);
        let password_key = identity_key(password, &salt, kdf)?;
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| VaultError::Random)?;
        let slots = RecoverySlots::create_with_recovery_key(
            &key,
            &password_key,
            &root_recovery_key(&self.root, &salt),
            &salt,
        )?;
        header.extend_from_slice(&slots.password);
        header.extend_from_slice(&slots.phrase);
        let plaintext =
            Zeroizing::new(serde_json::to_vec(self).expect("identity always serializes"));
        let ciphertext = format::cipher(&key)
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &header,
                },
            )
            .map_err(|_| VaultError::Corrupted)?;
        Ok((header, ciphertext))
    }
}

fn identity_key(
    password: &SecretString,
    salt: &[u8; 16],
    kdf: KdfParams,
) -> Result<Zeroizing<[u8; 32]>> {
    let (stretched, _) = format::derive(password, salt, kdf)?;
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(None, stretched.as_ref())
        .expand(b"lockbox.identity.v1\0", key.as_mut())
        .expect("fixed HKDF output size");
    Ok(key)
}

fn root_recovery_key(root: &[u8; 32], salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(salt), root)
        .expand(b"lockbox.identity-recovery.v1\0", key.as_mut())
        .expect("fixed HKDF output size");
    key
}

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
