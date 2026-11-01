//! Encrypted file format, authenticated contents and password derivation.

use crate::core::identity::Key;
use super::{KdfParams, Result, VaultError};
use crate::{
    core::{
        anchor::FileGeneration,
        entry::Entry,
        folder::{self, Folder},
    },
    infra::profile::BootstrapProfile,
};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"LBX1";
// v6/v7 preserve local record history and trash. Older clients must reject
// these files rather than silently discarding revisions on save.
pub(super) const VERSION: u16 = 6;
pub(super) const PROFILE_VERSION: u16 = 7;
pub(super) const SALT_LEN: usize = 16;
pub(super) const CHECK_LEN: usize = 32;
pub(super) const NONCE_LEN: usize = 24;
pub(super) const HEADER_LEN: usize = 4 + 2 + 12 + SALT_LEN + CHECK_LEN + NONCE_LEN;

const ENCRYPTION_INFO: &[u8] = b"lockbox v1 encryption";
const CHECK_INFO: &[u8] = b"lockbox v1 password check";

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Contents {
    pub(super) entries: Vec<Entry>,
    #[serde(default)]
    pub(super) history: Vec<crate::core::history::RecordChange>,
    #[serde(default)]
    pub(super) folders: Vec<Folder>,
    #[serde(default)]
    pub(super) profile: Option<BootstrapProfile>,
    #[serde(default)]
    pub(super) generation: Option<FileGeneration>,
}

/// Header fields needed to unlock and later re-save a vault.
pub(super) struct Header {
    pub(super) version: u16,
    pub(super) kdf: KdfParams,
    pub(super) salt: [u8; SALT_LEN],
    pub(super) check: [u8; CHECK_LEN],
}

impl Header {
    pub(super) fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_LEN || &data[..4] != MAGIC {
            return Err(VaultError::NotAVault);
        }

        let version = u16::from_le_bytes([data[4], data[5]]);

        if !(1..=PROFILE_VERSION).contains(&version) {
            return Err(VaultError::UnsupportedVersion(version));
        }

        let kdf = KdfParams {
            memory_kib: u32_at(data, 6),
            passes: u32_at(data, 10),
            lanes: u32_at(data, 14),
        }
        .validate()?;
        Ok(Self {
            version,
            kdf,
            salt: data[18..34].try_into().unwrap(),
            check: data[34..66].try_into().unwrap(),
        })
    }
}

/// Authenticated decryption of the whole file; `None` if the key or data is wrong.
pub(super) fn decrypt(data: &[u8], key: &Key) -> Option<Contents> {
    let (header, ciphertext) = data.split_at(HEADER_LEN);
    let nonce = XNonce::from_slice(&header[66..HEADER_LEN]);
    let plaintext = Zeroizing::new(
        cipher(key)
            .decrypt(
                nonce,
                Payload {
                    msg: ciphertext,
                    aad: header,
                },
            )
            .ok()?,
    );
    let contents: Contents = serde_json::from_slice(&plaintext).ok()?;

    for entry in &contents.entries {
        entry.validate_attachments().ok()?;
    }

    if contents.profile.as_ref().is_none_or(|p| p.sync.is_none()) {
        folder::validate(&contents.folders, &contents.entries).ok()?;
    }

    let version = u16::from_le_bytes([header[4], header[5]]);

    if (matches!(version, 3 | 5 | PROFILE_VERSION)) != contents.profile.is_some()
        || contents
            .profile
            .as_ref()
            .is_some_and(|profile| profile.requires_witness())
            != contents.generation.is_some()
        || contents
            .generation
            .is_some_and(|generation| !generation.valid())
        || contents
            .profile
            .as_ref()
            .is_some_and(|profile| !profile.validate())
    {
        return None;
    }
    Some(contents)
}

#[derive(Serialize)]
pub(super) struct ContentsRef<'a> {
    pub(super) entries: &'a [Entry],
    pub(super) history: &'a [crate::core::history::RecordChange],
    #[serde(skip_serializing_if = "<[Folder]>::is_empty")]
    pub(super) folders: &'a [Folder],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) profile: Option<&'a BootstrapProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) generation: Option<FileGeneration>,
}

/// Argon2id stretches the password; HKDF splits the result into the
/// encryption key and the stored password check value.
pub(super) fn derive(
    password: &SecretString,
    salt: &[u8; SALT_LEN],
    kdf: KdfParams,
) -> Result<(Zeroizing<Key>, [u8; CHECK_LEN])> {
    let params = Params::new(kdf.memory_kib, kdf.passes, kdf.lanes, Some(32))
        .map_err(|_| VaultError::InvalidKdfParams)?;
    let mut master = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.expose_secret().as_bytes(), salt, master.as_mut())
        .map_err(|_| VaultError::InvalidKdfParams)?;
    let hkdf = Hkdf::<Sha256>::new(None, master.as_ref());
    let mut key = Zeroizing::new([0u8; 32]);
    let mut check = [0u8; CHECK_LEN];
    hkdf.expand(ENCRYPTION_INFO, key.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    hkdf.expand(CHECK_INFO, &mut check)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    Ok((key, check))
}

pub(super) fn cipher(key: &Key) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.into())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

impl Header {
    pub(super) fn encode(
        kdf: KdfParams,
        salt: &[u8; SALT_LEN],
        check: &[u8; CHECK_LEN],
        shared: bool,
        nonce: &[u8; NONCE_LEN],
    ) -> [u8; HEADER_LEN] {
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(MAGIC);
        let version = if shared { PROFILE_VERSION } else { VERSION };
        header[4..6].copy_from_slice(&version.to_le_bytes());
        header[6..10].copy_from_slice(&kdf.memory_kib.to_le_bytes());
        header[10..14].copy_from_slice(&kdf.passes.to_le_bytes());
        header[14..18].copy_from_slice(&kdf.lanes.to_le_bytes());
        header[18..34].copy_from_slice(salt);
        header[34..66].copy_from_slice(check);
        header[66..].copy_from_slice(nonce);
        header
    }
}

pub(super) fn encrypt(
    contents: ContentsRef<'_>,
    key: &Key,
    header: &[u8; HEADER_LEN],
    nonce: &[u8; NONCE_LEN],
) -> Vec<u8> {
    let plaintext =
        Zeroizing::new(serde_json::to_vec(&contents).expect("vault entries always serialize"));
    cipher(key)
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &plaintext,
                aad: header,
            },
        )
        .expect("encryption of an in-memory buffer cannot fail")
}
