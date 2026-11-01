//! Encrypted file format, authenticated contents and password derivation.

use crate::{
    records::{
        entry::Entry,
        folder::{self, Folder},
        history::RecordChange,
    },
    sync::identity::Key,
    vault::{KdfParams, Result, VaultError, anchor::FileGeneration, profile::BootstrapProfile},
};
use argon2::{Algorithm, Argon2, Block, Params, Version};
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
// v8 local vaults and profiles add independent password and mnemonic key slots.
pub(super) const VERSION: u16 = 6;
pub(super) const PROFILE_VERSION: u16 = 7;
pub(super) const RECOVERY_VERSION: u16 = 8;
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
    pub(super) history: Vec<RecordChange>,
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
    pub(super) recovery: Option<crate::vault::recovery::RecoverySlots>,
    pub(super) kdf: KdfParams,
    pub(super) salt: [u8; SALT_LEN],
    pub(super) check: [u8; CHECK_LEN],
}

impl Header {
    pub(super) fn len(&self) -> usize {
        HEADER_LEN
            + if self.recovery.is_some() {
                crate::vault::recovery::SLOTS_LEN
            } else {
                0
            }
    }

    pub(super) fn password_key(&self, password: &SecretString) -> Result<Zeroizing<Key>> {
        use subtle::ConstantTimeEq;
        let (key, check) = derive(password, &self.salt, self.kdf)?;
        if !bool::from(check.ct_eq(&self.check)) {
            return Err(VaultError::WrongPassword);
        }
        match &self.recovery {
            Some(slots) => slots.password_key(&key, &self.salt),
            None => Ok(key),
        }
    }

    pub(super) fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_LEN || &data[..4] != MAGIC {
            return Err(VaultError::NotAVault);
        }

        let version = u16::from_le_bytes([data[4], data[5]]);

        if !(1..=RECOVERY_VERSION).contains(&version) {
            return Err(VaultError::UnsupportedVersion(version));
        }

        let kdf = KdfParams {
            memory_kib: u32_at(data, 6),
            passes: u32_at(data, 10),
            lanes: u32_at(data, 14),
        }
        .validate()?;
        let recovery = if version == RECOVERY_VERSION {
            let slots = data
                .get(HEADER_LEN..HEADER_LEN + crate::vault::recovery::SLOTS_LEN)
                .ok_or(VaultError::NotAVault)?;
            Some(crate::vault::recovery::RecoverySlots {
                password: slots[..crate::vault::recovery::SLOT_LEN]
                    .try_into()
                    .unwrap(),
                phrase: slots[crate::vault::recovery::SLOT_LEN..]
                    .try_into()
                    .unwrap(),
            })
        } else {
            None
        };
        let len = HEADER_LEN
            + if recovery.is_some() {
                crate::vault::recovery::SLOTS_LEN
            } else {
                0
            };
        if data.len() < len + 16 {
            return Err(VaultError::NotAVault);
        }
        Ok(Self {
            recovery,
            version,
            kdf,
            salt: data[18..34].try_into().unwrap(),
            check: data[34..66].try_into().unwrap(),
        })
    }
}

/// Authenticated decryption of the whole file; `None` if the key or data is wrong.
pub(super) fn decrypt(data: &[u8], key: &Key) -> Option<Contents> {
    let parsed = Header::parse(data).ok()?;
    let (header, ciphertext) = data.split_at(parsed.len());
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

    // Recovery key slots work with either local records or a shared profile.
    if (version != RECOVERY_VERSION
        && matches!(version, 3 | 5 | PROFILE_VERSION) != contents.profile.is_some())
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
    #[serde(serialize_with = "serialize_history")]
    pub(super) history: [&'a [RecordChange]; 2],
    #[serde(skip_serializing_if = "<[Folder]>::is_empty")]
    pub(super) folders: &'a [Folder],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) profile: Option<&'a BootstrapProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) generation: Option<FileGeneration>,
}

/// Existing and pending revisions retain one flat history array on disk.
fn serialize_history<S: serde::Serializer>(
    history: &[&[RecordChange]; 2],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_seq(history.iter().flat_map(|chunk| chunk.iter()))
}

/// Argon2id stretches the password; HKDF splits the result into the
/// encryption key and the stored password check value.
pub(super) fn derive(
    password: &SecretString,
    salt: &[u8; SALT_LEN],
    kdf: KdfParams,
) -> Result<(Zeroizing<Key>, [u8; CHECK_LEN])> {
    let master = stretch(password, salt, kdf)?;
    let hkdf = Hkdf::<Sha256>::new(None, master.as_ref());
    let mut key = Zeroizing::new([0u8; 32]);
    let mut check = [0u8; CHECK_LEN];
    hkdf.expand(ENCRYPTION_INFO, key.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 length");
    hkdf.expand(CHECK_INFO, &mut check)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    Ok((key, check))
}

/// Wipe the complete Argon2 arena, including error paths, rather than leaving
/// password-derived blocks in an ordinary allocator buffer after unlock.
pub(super) fn stretch(
    password: &SecretString,
    salt: &[u8],
    kdf: KdfParams,
) -> Result<Zeroizing<Key>> {
    let kdf = kdf.validate()?;
    let params = Params::new(kdf.memory_kib, kdf.passes, kdf.lanes, Some(32))
        .map_err(|_| VaultError::InvalidKdfParams)?;
    let count = params.block_count();
    let mut blocks = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(count)
        .map_err(|_| VaultError::ResourceLimit)?;
    blocks.resize(count, Block::default());
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            password.expose_secret().as_bytes(),
            salt,
            key.as_mut(),
            blocks.as_mut_slice(),
        )
        .map_err(|_| VaultError::InvalidKdfParams)?;
    Ok(key)
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
    header: &[u8],
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
