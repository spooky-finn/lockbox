//! Encrypted vault file.
//!
//! Layout (little-endian), 90-byte header followed by the ciphertext:
//!
//! | offset | size | field                                   |
//! |--------|------|-----------------------------------------|
//! | 0      | 4    | magic `LBX1`                            |
//! | 4      | 2    | format version                          |
//! | 6      | 12   | Argon2id memory (KiB), passes, lanes    |
//! | 18     | 16   | salt                                    |
//! | 34     | 32   | password check value                    |
//! | 66     | 24   | XChaCha20-Poly1305 nonce                |
//!
//! The whole header is authenticated as associated data, so no field can be
//! changed without the vault failing to open.
//!
//! Shared-space commands live in `shared`; receive commits live in `exchange`.
//! This file owns encryption, password derivation, process lock, and atomic replace.
//! The public [`Vault`] facade retains the existing import paths for both modes.

use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
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
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::profile::BootstrapProfile;
pub use super::profile::ProviderConfiguration;

use crate::{
    core::anchor::{self, FileGeneration},
    core::entry::Entry,
    core::folder::{self, Folder},
    infra::witness::{self, WitnessStore},
};

const MAGIC: &[u8; 4] = b"LBX1";
// v2 adds folders. Accept v1 on read, but keep old clients from silently
// discarding folder metadata when they re-save a new vault.
const VERSION: u16 = 2;
const PROFILE_VERSION: u16 = 3;
const SALT_LEN: usize = 16;
const CHECK_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 2 + 12 + SALT_LEN + CHECK_LEN + NONCE_LEN;

const ENCRYPTION_INFO: &[u8] = b"lockbox v1 encryption";
const CHECK_INFO: &[u8] = b"lockbox v1 password check";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("vault file is unavailable: {0}")]
    Io(#[from] io::Error),
    #[error("could not obtain random data from the operating system")]
    Random,
    #[error("profile is already open in another process")]
    ProfileInUse,
    #[error("vault already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("this is not a lockbox file or it is truncated")]
    NotAVault,
    #[error("format version {0} is not supported")]
    UnsupportedVersion(u16),
    #[error("invalid Argon2id parameters in the header")]
    InvalidKdfParams,
    #[error("incorrect master password")]
    WrongPassword,
    #[error("vault file is corrupted")]
    Corrupted,
    #[error("saved key does not match this file — open the vault with the master password")]
    KeyMismatch,
    #[error("invalid folder structure: {0}")]
    InvalidFolders(&'static str),
    #[error("sync: {0}")]
    Sync(#[from] crate::core::sync::Error),
    #[error("Storj: {0}")]
    Transport(#[from] crate::infra::sync::TransportError),
    #[error("could not unlock the root key: check the recovery password")]
    RecoveryFailed,
    #[error("received changes await a durable commit; retry synchronization")]
    PendingSyncCommit,
    #[error("{0}")]
    ProfileProtection(#[from] anchor::Error),
}

pub type Result<T> = std::result::Result<T, VaultError>;

/// Argon2id cost. Stored in the header so stronger settings can be adopted later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Default for KdfParams {
    /// About half a second to unlock on a current laptop.
    fn default() -> Self {
        Self {
            memory_kib: 256 * 1024,
            passes: 3,
            lanes: 4,
        }
    }
}

impl KdfParams {
    /// Bounds checked before running Argon2id, so a crafted header cannot
    /// make the app allocate gigabytes or spin for minutes.
    pub(crate) fn validate(self) -> Result<Self> {
        let ok = (8 * 1024..=4 * 1024 * 1024).contains(&self.memory_kib)
            && (1..=20).contains(&self.passes)
            && (1..=16).contains(&self.lanes);
        ok.then_some(self).ok_or(VaultError::InvalidKdfParams)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Contents {
    entries: Vec<Entry>,
    #[serde(default)]
    folders: Vec<Folder>,
    #[serde(default)]
    profile: Option<BootstrapProfile>,
    #[serde(default)]
    generation: Option<FileGeneration>,
}

/// An unlocked vault. The derived key and all entries are wiped when it is dropped.
pub struct Vault {
    path: PathBuf,
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    check: [u8; CHECK_LEN],
    key: Zeroizing<[u8; 32]>,
    pub entries: Vec<Entry>,
    pub folders: Vec<Folder>,
    pub(crate) profile: Option<BootstrapProfile>,
    pending_exchange: Option<exchange::PendingExchange>,
    _profile_lock: Option<File>,
    witnesses: Arc<dyn WitnessStore>,
    protection: Option<RefCell<protection::Protection>>,
}

mod exchange;
mod protection;
mod shared;
mod store;

pub use store::{DeviceRecovery, ProfileStore, SharedProfile};

enum Unlock<'a> {
    Password(&'a SecretString),
    Key(&'a [u8; 32]),
}

impl Vault {
    /// Creates and saves an empty vault. Never overwrites an existing file.
    pub fn create(path: impl AsRef<Path>, password: &SecretString, kdf: KdfParams) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            return Err(VaultError::AlreadyExists(path));
        }
        let kdf = kdf.validate()?;
        let mut salt = [0u8; SALT_LEN];
        random(&mut salt);
        let (key, check) = derive(password, &salt, kdf)?;
        let vault = Self {
            path,
            kdf,
            salt,
            check,
            key,
            entries: Vec::new(),
            folders: Vec::new(),
            profile: None,
            pending_exchange: None,
            _profile_lock: None,
            witnesses: witness::system(),
            protection: None,
        };
        vault.save()?;
        Ok(vault)
    }

    /// Saves device-local bootstrap state. This file is never a shared sync object.
    pub(crate) fn create_profile(
        path: impl AsRef<Path>,
        password: &SecretString,
        kdf: KdfParams,
        profile: BootstrapProfile,
    ) -> Result<Self> {
        ProfileStore::default().create_profile(path, password, kdf, profile)
    }

    fn create_profile_using(
        path: &Path,
        password: &SecretString,
        kdf: KdfParams,
        profile: BootstrapProfile,
        witnesses: Arc<dyn WitnessStore>,
    ) -> Result<Self> {
        let path = path.to_path_buf();
        if path.exists() {
            return Err(VaultError::AlreadyExists(path));
        }
        if !profile.validate() {
            return Err(VaultError::Corrupted);
        }
        let profile_lock = lock_profile(&path)?;
        let kdf = kdf.validate()?;
        let mut salt = [0; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|_| VaultError::Random)?;
        let (key, check) = derive(password, &salt, kdf)?;
        let protection = if profile.requires_witness() {
            Some(RefCell::new(protection::Protection::create(
                witnesses.clone(),
                profile.sync_secret().id(),
                &salt,
            )?))
        } else {
            None
        };
        let vault = Self {
            path,
            kdf,
            salt,
            check,
            key,
            entries: Vec::new(),
            folders: Vec::new(),
            profile: Some(profile),
            pending_exchange: None,
            _profile_lock: Some(profile_lock),
            witnesses,
            protection,
        };
        vault.save_inner(true)?;
        Ok(vault)
    }

    pub fn open(path: impl AsRef<Path>, password: &SecretString) -> Result<Self> {
        ProfileStore::default().open(path, password)
    }

    /// Opens with a key exported earlier, skipping Argon2id (used for Touch ID).
    pub fn open_with_key(path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        ProfileStore::default().open_with_key(path, key)
    }

    fn open_using(
        path: &Path,
        unlock: Unlock<'_>,
        witnesses: Arc<dyn WitnessStore>,
    ) -> Result<Self> {
        let path = path.to_path_buf();
        let data = fs::read(&path)?;
        let header = Header::parse(&data)?;
        let path = if header.version == PROFILE_VERSION {
            path.canonicalize()?
        } else {
            path
        };
        let profile_lock = if header.version == PROFILE_VERSION {
            Some(lock_profile(&path)?)
        } else {
            None
        };
        let (data, header) = refresh_profile_snapshot(&path, data, header, &profile_lock)?;
        let (key, failed_decryption) = match unlock {
            Unlock::Password(password) => {
                let (key, check) = derive(password, &header.salt, header.kdf)?;
                if !bool::from(check.ct_eq(&header.check)) {
                    return Err(VaultError::WrongPassword);
                }
                (key, VaultError::Corrupted)
            }
            Unlock::Key(key) => (Zeroizing::new(*key), VaultError::KeyMismatch),
        };
        let contents = decrypt(&data, &key).ok_or(failed_decryption)?;
        let generation = contents.generation;
        let mut vault = header.into_vault(path, key, contents, profile_lock, witnesses.clone());
        if let Some(generation) = generation {
            let device = vault
                .profile
                .as_ref()
                .ok_or(VaultError::Corrupted)?
                .sync_secret()
                .id();
            let digest = protection::digest_parts(&data[..HEADER_LEN], &data[HEADER_LEN..]);
            vault.protection = Some(RefCell::new(protection::Protection::open(
                witnesses,
                device,
                &vault.salt,
                generation,
                digest,
                &vault.path,
            )?));
        }
        if vault.ensure_profile_writable().is_ok() {
            vault.recover_sync()?;
        }
        Ok(vault)
    }

    /// Stable identifier of the vault file: its random salt, which never changes.
    pub fn id(&self) -> String {
        hex(&self.salt)
    }

    /// Reads the identifier without the password, to find a saved Touch ID key.
    pub fn peek_id(path: impl AsRef<Path>) -> Result<String> {
        let data = fs::read(path)?;
        Ok(hex(&Header::parse(&data)?.salt))
    }

    /// The derived encryption key, for storing behind Touch ID.
    pub fn export_key(&self) -> Zeroizing<[u8; 32]> {
        self.key.clone()
    }

    /// Encrypts with a fresh nonce and replaces the file atomically,
    /// keeping the previous version next to it as `<name>.bak`.
    pub fn save(&self) -> Result<()> {
        self.ensure_profile_writable()?;
        self.ensure_sync_projection()?;
        self.save_inner(false)
    }

    fn save_inner(&self, create_new: bool) -> Result<()> {
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| !profile.validate())
        {
            return Err(VaultError::Corrupted);
        }
        if self.profile.as_ref().is_none_or(|p| p.sync.is_none()) {
            folder::validate(&self.folders, &self.entries).map_err(VaultError::InvalidFolders)?;
        }
        let requires_witness = self.profile.as_ref().is_some_and(|p| p.requires_witness());
        if !requires_witness && self.protection.is_some() {
            return Err(VaultError::Corrupted);
        }
        let mut protection = self.protection.as_ref().map(RefCell::borrow_mut);
        let generation = if requires_witness {
            Some(
                protection
                    .as_mut()
                    .ok_or(anchor::Error::Legacy)?
                    .prepare_save(&self.path)?,
            )
        } else if self.profile.as_ref().is_some_and(|p| p.sync.is_some()) {
            return Err(anchor::Error::Legacy.into());
        } else {
            None
        };
        let mut nonce = [0u8; NONCE_LEN];
        random(&mut nonce);
        let header = self.header(&nonce);
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&ContentsRef {
                entries: &self.entries,
                folders: &self.folders,
                profile: self.profile.as_ref(),
                generation,
            })
            .expect("vault entries always serialize"),
        );
        let ciphertext = cipher(&self.key)
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &header,
                },
            )
            .expect("encryption of an in-memory buffer cannot fail");

        if create_new {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&self.path).map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    VaultError::AlreadyExists(self.path.clone())
                } else {
                    VaultError::Io(error)
                }
            })?;
            let result = (|| {
                file.write_all(&header)?;
                file.write_all(&ciphertext)?;
                file.sync_all()
            })();
            if result.is_err() {
                let _ = fs::remove_file(&self.path);
            }
            result?;
            if let (Some(protection), Some(generation)) = (protection.as_mut(), generation) {
                protection.committed(generation, protection::digest_parts(&header, &ciphertext));
            }
            sync_dir(&self.path)?;
            if let Some(protection) = protection.as_mut() {
                protection.ensure_ready(&self.path)?;
            }
            return Ok(());
        }
        let tmp = sibling(&self.path, "tmp");
        {
            let mut file = private_file(&tmp)?;
            file.write_all(&header)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
        }
        if self.path.exists() {
            let backup = sibling(&self.path, "bak");
            let backup_tmp = sibling(&backup, "tmp");
            let mut source = File::open(&self.path)?;
            let mut target = private_file(&backup_tmp)?;
            io::copy(&mut source, &mut target)?;
            target.sync_all()?;
            fs::rename(&backup_tmp, &backup)?;
        }
        fs::rename(&tmp, &self.path)?;
        if let (Some(protection), Some(generation)) = (protection.as_mut(), generation) {
            protection.committed(generation, protection::digest_parts(&header, &ciphertext));
        }
        sync_dir(&self.path)?;
        if let Some(protection) = protection.as_mut() {
            protection.ensure_ready(&self.path)?;
        }
        Ok(())
    }

    /// Cached reason this shared profile cannot sign or exchange. Readable data
    /// remains available. Missing bindings are never recreated implicitly.
    pub fn signing_error(&self) -> Option<anchor::Error> {
        if self.profile.as_ref().is_none_or(|p| p.sync.is_none()) {
            return None;
        }
        match &self.protection {
            Some(protection) => {
                let protection = protection.borrow();
                protection
                    .error
                    .or_else(|| (!protection.has_file()).then_some(anchor::Error::Unavailable))
            }
            None => Some(anchor::Error::Legacy),
        }
    }

    /// Retry an unavailable credential store or reconcile exactly one linked
    /// file generation. Never enroll an imported or rolled-back signer.
    pub fn reconcile_profile(&self) -> Result<()> {
        self.ensure_profile_writable()
    }

    pub(crate) fn ensure_profile_writable(&self) -> Result<()> {
        if self.profile.as_ref().is_none_or(|p| p.sync.is_none()) {
            return Ok(());
        }
        self.protection
            .as_ref()
            .ok_or(anchor::Error::Legacy)?
            .borrow_mut()
            .ensure_ready(&self.path)?;
        Ok(())
    }

    fn ensure_sync_commit_allowed(&self) -> Result<()> {
        self.protection
            .as_ref()
            .ok_or(anchor::Error::Legacy)?
            .borrow_mut()
            .prepare_save(&self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn header(&self, nonce: &[u8; NONCE_LEN]) -> [u8; HEADER_LEN] {
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(MAGIC);
        let version = if self.profile.is_some() {
            PROFILE_VERSION
        } else {
            VERSION
        };
        header[4..6].copy_from_slice(&version.to_le_bytes());
        header[6..10].copy_from_slice(&self.kdf.memory_kib.to_le_bytes());
        header[10..14].copy_from_slice(&self.kdf.passes.to_le_bytes());
        header[14..18].copy_from_slice(&self.kdf.lanes.to_le_bytes());
        header[18..34].copy_from_slice(&self.salt);
        header[34..66].copy_from_slice(&self.check);
        header[66..].copy_from_slice(nonce);
        header
    }
}

/// Header fields needed to unlock and later re-save a vault.
struct Header {
    version: u16,
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    check: [u8; CHECK_LEN],
}

impl Header {
    fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_LEN || &data[..4] != MAGIC {
            return Err(VaultError::NotAVault);
        }
        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != 1 && version != VERSION && version != PROFILE_VERSION {
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

    fn into_vault(
        self,
        path: PathBuf,
        key: Zeroizing<[u8; 32]>,
        contents: Contents,
        profile_lock: Option<File>,
        witnesses: Arc<dyn WitnessStore>,
    ) -> Vault {
        Vault {
            path,
            kdf: self.kdf,
            salt: self.salt,
            check: self.check,
            key,
            entries: contents.entries,
            folders: contents.folders,
            profile: contents.profile,
            pending_exchange: None,
            _profile_lock: profile_lock,
            witnesses,
            protection: None,
        }
    }
}

/// Authenticated decryption of the whole file; `None` if the key or data is wrong.
fn decrypt(data: &[u8], key: &[u8; 32]) -> Option<Contents> {
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
    if contents.profile.as_ref().is_none_or(|p| p.sync.is_none()) {
        folder::validate(&contents.folders, &contents.entries).ok()?;
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if (version == PROFILE_VERSION) != contents.profile.is_some()
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Serialize)]
struct ContentsRef<'a> {
    entries: &'a [Entry],
    #[serde(skip_serializing_if = "<[Folder]>::is_empty")]
    folders: &'a [Folder],
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<&'a BootstrapProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<FileGeneration>,
}

/// Argon2id stretches the password; HKDF splits the result into the
/// encryption key and the stored password check value.
fn derive(
    password: &SecretString,
    salt: &[u8; SALT_LEN],
    kdf: KdfParams,
) -> Result<(Zeroizing<[u8; 32]>, [u8; CHECK_LEN])> {
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

fn cipher(key: &[u8; 32]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.into())
}

fn random(buffer: &mut [u8]) {
    getrandom::fill(buffer).expect("operating system random generator");
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn sibling(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(extension);
    path.with_file_name(name)
}

fn refresh_profile_snapshot(
    path: &Path,
    data: Vec<u8>,
    header: Header,
    lock: &Option<File>,
) -> Result<(Vec<u8>, Header)> {
    if lock.is_none() {
        return Ok((data, header));
    }
    let data = fs::read(path)?;
    let header = Header::parse(&data)?;
    if header.version != PROFILE_VERSION {
        return Err(VaultError::Corrupted);
    }
    Ok((data, header))
}

/// Stable sidecar: replacing the encrypted file must not release the process lock.
fn lock_profile(path: &Path) -> Result<File> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical =
        parent.canonicalize()?.join(path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "profile filename required")
        })?);
    let canonical = if path.exists() {
        path.canonicalize()?
    } else {
        canonical
    };
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(sibling(&canonical, "lock"))?;
    file.try_lock().map_err(|_| VaultError::ProfileInUse)?;
    Ok(file)
}

/// Readable and writable by the owner only.
fn private_file(path: &Path) -> io::Result<File> {
    // Unlink stale sidecars themselves, then exclusively create the new file.
    // Never truncate a symlink target through a predictable temporary pathname.
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Makes the rename itself durable, not only the file contents.
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        File::open(dir.unwrap_or(Path::new(".")))?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/vault.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/space_sync.rs"]
mod sync_tests;

#[cfg(test)]
#[path = "tests/protection.rs"]
mod protection_tests;
