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
//! Format/crypto live in `format`; process locks and replacement live in `file`.
//! This facade keeps file-generation and witness commit ordering explicit.
//! The public [`Vault`] facade retains the existing import paths for both modes.

use std::{
    cell::RefCell,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::profile::BootstrapProfile;
pub use super::profile::ProviderConfiguration;

use crate::{
    core::anchor::{self},
    core::entry::Entry,
    core::folder::{self, Folder},
    infra::witness::{self, WitnessStore},
};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("vault file is unavailable: {0}")]
    Io(#[from] io::Error),
    #[error("could not obtain random data from the operating system")]
    Random,
    #[error("vault is already open in another window or process")]
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

/// An unlocked vault. The derived key and all entries are wiped when it is dropped.
pub struct Vault {
    path: PathBuf,
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    check: [u8; CHECK_LEN],
    key: Zeroizing<[u8; 32]>,
    pub entries: Vec<Entry>,
    history: RefCell<Vec<crate::core::history::RecordChange>>,
    saved_entries: RefCell<Vec<Entry>>,
    pub folders: Vec<Folder>,
    pub(crate) profile: Option<BootstrapProfile>,
    pending_exchange: Option<exchange::PendingExchange>,
    _profile_lock: Option<crate::infra::lease::ExclusiveFile>,
    witnesses: Arc<dyn WitnessStore>,
    protection: Option<RefCell<protection::Protection>>,
}

mod exchange;
mod file;
mod format;

use file::{lock_profile, refresh_profile_snapshot, sync_dir};
use format::{
    CHECK_LEN, Contents, ContentsRef, HEADER_LEN, Header, NONCE_LEN, PROFILE_VERSION, SALT_LEN,
    decrypt, derive,
};
mod authority;
mod history;
mod protection;
mod records;
mod root_journal;
mod shared;
mod sharing;
mod store;

pub use crate::infra::profile::{AcceptedInvitationInfo, InvitationStatus, IssuedInvitationInfo};
pub use records::Records;
pub use store::{ControllerRecovery, DeviceRecovery, ProfileStore, SharedProfile};

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

        // Local vaults are locked like profiles: two windows editing one file
        // would otherwise silently overwrite each other's saves.
        let file_lock = lock_profile(&path)?;
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
            history: RefCell::default(),
            saved_entries: RefCell::default(),
            folders: Vec::new(),
            profile: None,
            pending_exchange: None,
            _profile_lock: Some(file_lock),
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
        Self::create_profile_snapshot(
            path,
            password,
            kdf,
            profile,
            witnesses,
            (Vec::new(), Vec::new()),
        )
    }

    fn create_profile_snapshot(
        path: &Path,
        password: &SecretString,
        kdf: KdfParams,
        profile: BootstrapProfile,
        witnesses: Arc<dyn WitnessStore>,
        snapshot: (Vec<Entry>, Vec<crate::core::folder::Folder>),
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
            history: RefCell::default(),
            saved_entries: RefCell::new(snapshot.0.clone()),
            entries: snapshot.0,
            folders: snapshot.1,
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
        let path = if matches!(header.version, 3 | 5 | PROFILE_VERSION) {
            path.canonicalize()?
        } else {
            path
        };

        let profile_lock = if matches!(header.version, 3 | 5 | PROFILE_VERSION) {
            Some(lock_profile(&path)?)
        } else {
            None
        };

        let (data, header) = refresh_profile_snapshot(&path, data, header, &profile_lock)?;
        // Profiles re-read their snapshot under the lock above; a local vault
        // takes the same sidecar lock so another window cannot open it too.
        let profile_lock = match profile_lock {
            Some(lock) => Some(lock),
            None => Some(lock_profile(&path)?),
        };
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
        for entry in &self.entries {
            entry
                .validate_attachments()
                .map_err(|_| VaultError::Corrupted)?;
        }

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
        let header = Header::encode(
            self.kdf,
            &self.salt,
            &self.check,
            self.profile.is_some(),
            &nonce,
        );
        let history = self.next_local_history();
        let ciphertext = format::encrypt(
            ContentsRef {
                entries: &self.entries,
                history: &history,
                folders: &self.folders,
                profile: self.profile.as_ref(),
                generation,
            },
            &self.key,
            &header,
            &nonce,
        );

        if create_new {
            file::create(&self.path, &header, &ciphertext)?;
        } else {
            file::replace(&self.path, &header, &ciphertext)?;
        }

        if let (Some(protection), Some(generation)) = (protection.as_mut(), generation) {
            protection.committed(generation, protection::digest_parts(&header, &ciphertext));
        }
        *self.history.borrow_mut() = history;
        *self.saved_entries.borrow_mut() = self.entries.clone();
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
}

impl Header {
    fn into_vault(
        self,
        path: PathBuf,
        key: Zeroizing<[u8; 32]>,
        contents: Contents,
        profile_lock: Option<crate::infra::lease::ExclusiveFile>,
        witnesses: Arc<dyn WitnessStore>,
    ) -> Vault {
        Vault {
            path,
            kdf: self.kdf,
            salt: self.salt,
            check: self.check,
            key,
            history: RefCell::new(contents.history),
            saved_entries: RefCell::new(contents.entries.clone()),
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn random(buffer: &mut [u8]) {
    getrandom::fill(buffer).expect("operating system random generator");
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

#[cfg(test)]
#[path = "tests/sharing.rs"]
mod sharing_tests;

#[cfg(test)]
#[path = "tests/authority.rs"]
mod authority_tests;
