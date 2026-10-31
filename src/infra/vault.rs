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

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
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

use crate::{
    core::entry::Entry,
    core::folder::{self, Folder},
};

const MAGIC: &[u8; 4] = b"LBX1";
// v2 adds folders. Accept v1 on read, but keep old clients from silently
// discarding folder metadata when they re-save a new vault.
const VERSION: u16 = 2;
const SALT_LEN: usize = 16;
const CHECK_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 2 + 12 + SALT_LEN + CHECK_LEN + NONCE_LEN;

const ENCRYPTION_INFO: &[u8] = b"lockbox v1 encryption";
const CHECK_INFO: &[u8] = b"lockbox v1 password check";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("файл хранилища недоступен: {0}")]
    Io(#[from] io::Error),
    #[error("хранилище уже существует: {0}")]
    AlreadyExists(PathBuf),
    #[error("это не файл lockbox или он обрезан")]
    NotAVault,
    #[error("версия формата {0} не поддерживается")]
    UnsupportedVersion(u16),
    #[error("недопустимые параметры Argon2id в заголовке")]
    InvalidKdfParams,
    #[error("неверный мастер-пароль")]
    WrongPassword,
    #[error("файл хранилища повреждён")]
    Corrupted,
    #[error("сохранённый ключ не подходит к этому файлу — откройте хранилище мастер-паролем")]
    KeyMismatch,
    #[error("неверная структура папок: {0}")]
    InvalidFolders(&'static str),
}

pub type Result<T> = std::result::Result<T, VaultError>;

/// Argon2id cost. Stored in the header so stronger settings can be adopted later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn validate(self) -> Result<Self> {
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
        };
        vault.save()?;
        Ok(vault)
    }

    pub fn open(path: impl AsRef<Path>, password: &SecretString) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data = fs::read(&path)?;
        let header = Header::parse(&data)?;
        let (key, check) = derive(password, &header.salt, header.kdf)?;
        // The check value tells a mistyped password apart from a damaged file.
        if !bool::from(check.ct_eq(&header.check)) {
            return Err(VaultError::WrongPassword);
        }
        let contents = decrypt(&data, &key).ok_or(VaultError::Corrupted)?;
        Ok(header.into_vault(path, key, contents))
    }

    /// Opens with a key exported earlier, skipping Argon2id (used for Touch ID).
    pub fn open_with_key(path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data = fs::read(&path)?;
        let header = Header::parse(&data)?;
        let key = Zeroizing::new(*key);
        let contents = decrypt(&data, &key).ok_or(VaultError::KeyMismatch)?;
        Ok(header.into_vault(path, key, contents))
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
        folder::validate(&self.folders, &self.entries).map_err(VaultError::InvalidFolders)?;
        let mut nonce = [0u8; NONCE_LEN];
        random(&mut nonce);
        let header = self.header(&nonce);
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&ContentsRef {
                entries: &self.entries,
                folders: &self.folders,
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

        let tmp = sibling(&self.path, "tmp");
        {
            let mut file = private_file(&tmp)?;
            file.write_all(&header)?;
            file.write_all(&ciphertext)?;
            file.sync_all()?;
        }
        if self.path.exists() {
            let backup = sibling(&self.path, "bak");
            fs::copy(&self.path, &backup)?;
            File::open(&backup)?.sync_all()?;
        }
        fs::rename(&tmp, &self.path)?;
        sync_dir(&self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn header(&self, nonce: &[u8; NONCE_LEN]) -> [u8; HEADER_LEN] {
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(MAGIC);
        header[4..6].copy_from_slice(&VERSION.to_le_bytes());
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
        if version != 1 && version != VERSION {
            return Err(VaultError::UnsupportedVersion(version));
        }
        let kdf = KdfParams {
            memory_kib: u32_at(data, 6),
            passes: u32_at(data, 10),
            lanes: u32_at(data, 14),
        }
        .validate()?;
        Ok(Self {
            kdf,
            salt: data[18..34].try_into().unwrap(),
            check: data[34..66].try_into().unwrap(),
        })
    }

    fn into_vault(self, path: PathBuf, key: Zeroizing<[u8; 32]>, contents: Contents) -> Vault {
        Vault {
            path,
            kdf: self.kdf,
            salt: self.salt,
            check: self.check,
            key,
            entries: contents.entries,
            folders: contents.folders,
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
    folder::validate(&contents.folders, &contents.entries).ok()?;
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

/// Readable and writable by the owner only.
fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
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
mod tests {
    use super::*;

    /// Cheap Argon2id settings: the tests check the format, not the cost.
    const FAST: KdfParams = KdfParams {
        memory_kib: 8 * 1024,
        passes: 1,
        lanes: 1,
    };

    fn password(value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }

    fn sample_entry() -> Entry {
        let mut entry = Entry::new();
        entry.title = "GitLab".into();
        entry.username = "a8ee".into();
        entry.password = "correct horse battery staple".into();
        entry.url = "https://gitlab.example".into();
        entry.notes = "work".into();
        entry.totp = Some(crate::core::entry::Totp {
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: crate::core::entry::TotpAlgorithm::Sha1,
            digits: 6,
            period: 30,
        });
        entry
    }

    fn new_vault(dir: &tempfile::TempDir) -> (PathBuf, Vault) {
        let path = dir.path().join("test.lbx");
        let mut vault = Vault::create(&path, &password("master"), FAST).unwrap();
        vault.entries.push(sample_entry());
        vault.save().unwrap();
        (path, vault)
    }

    fn flip_byte(path: &Path, offset: usize) {
        let mut data = fs::read(path).unwrap();
        data[offset] ^= 0x01;
        fs::write(path, data).unwrap();
    }

    #[test]
    fn saved_entries_open_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (path, vault) = new_vault(&dir);
        let opened = Vault::open(&path, &password("master")).unwrap();
        assert_eq!(opened.entries, vault.entries);
        assert_eq!(opened.kdf, FAST);
    }

    #[test]
    fn vaults_written_before_folders_open_without_migration() {
        let dir = tempfile::tempdir().unwrap();
        let (path, vault) = new_vault(&dir);
        let legacy = serde_json::json!({"entries": [{
            "id": "legacy-id", "title": "Legacy", "username": "alice",
            "password": "preserved", "url": "", "notes": "", "totp": null
        }]});
        let nonce = [3u8; NONCE_LEN];
        let mut header = vault.header(&nonce);
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        let plaintext = Zeroizing::new(serde_json::to_vec(&legacy).unwrap());
        let encrypted = cipher(&vault.key)
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &header,
                },
            )
            .unwrap();
        let mut data = header.to_vec();
        data.extend(encrypted);
        fs::write(&path, data).unwrap();
        let opened = Vault::open(&path, &password("master")).unwrap();
        assert!(opened.folders.is_empty());
        assert!(opened.entries[0].folder_id.is_none());
        assert_eq!(opened.entries[0].password, "preserved");
        opened.save().unwrap();
        let upgraded = fs::read(&path).unwrap();
        assert_eq!(u16::from_le_bytes([upgraded[4], upgraded[5]]), VERSION);
        assert_eq!(
            Vault::open_with_key(&path, &opened.export_key())
                .unwrap()
                .entries,
            opened.entries
        );
    }

    #[test]
    fn wrong_password_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = new_vault(&dir);
        assert!(matches!(
            Vault::open(&path, &password("Master")),
            Err(VaultError::WrongPassword)
        ));
    }

    #[test]
    fn damaged_ciphertext_or_nonce_is_reported_as_corruption() {
        for offset in [HEADER_LEN + 3, 70] {
            let dir = tempfile::tempdir().unwrap();
            let (path, _) = new_vault(&dir);
            flip_byte(&path, offset);
            assert!(matches!(
                Vault::open(&path, &password("master")),
                Err(VaultError::Corrupted)
            ));
        }
    }

    #[test]
    fn changed_salt_or_check_value_never_opens() {
        for offset in [20, 40] {
            let dir = tempfile::tempdir().unwrap();
            let (path, _) = new_vault(&dir);
            flip_byte(&path, offset);
            assert!(Vault::open(&path, &password("master")).is_err());
        }
    }

    #[test]
    fn foreign_or_truncated_files_are_not_vaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.lbx");
        fs::write(&path, b"not a vault").unwrap();
        assert!(matches!(
            Vault::open(&path, &password("master")),
            Err(VaultError::NotAVault)
        ));
        let (path, _) = new_vault(&dir);
        flip_byte(&path, 0);
        assert!(matches!(
            Vault::open(&path, &password("master")),
            Err(VaultError::NotAVault)
        ));
    }

    #[test]
    fn oversized_kdf_params_are_rejected_before_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = new_vault(&dir);
        let mut data = fs::read(&path).unwrap();
        data[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&path, data).unwrap();
        assert!(matches!(
            Vault::open(&path, &password("master")),
            Err(VaultError::InvalidKdfParams)
        ));
    }

    #[test]
    fn every_save_uses_a_new_nonce() {
        let dir = tempfile::tempdir().unwrap();
        let (path, vault) = new_vault(&dir);
        let first = fs::read(&path).unwrap();
        vault.save().unwrap();
        let second = fs::read(&path).unwrap();
        assert_ne!(first[66..HEADER_LEN], second[66..HEADER_LEN]);
        assert_ne!(first[HEADER_LEN..], second[HEADER_LEN..]);
    }

    #[test]
    fn previous_version_is_kept_as_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut vault) = new_vault(&dir);
        vault.entries.clear();
        vault.save().unwrap();
        let backup = Vault::open(sibling(&path, "bak"), &password("master")).unwrap();
        assert_eq!(backup.entries.len(), 1);
        assert!(
            Vault::open(&path, &password("master"))
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(!sibling(&path, "tmp").exists());
    }

    #[test]
    fn create_never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = new_vault(&dir);
        assert!(matches!(
            Vault::create(&path, &password("other"), FAST),
            Err(VaultError::AlreadyExists(_))
        ));
        assert_eq!(
            Vault::open(&path, &password("master"))
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn exported_key_opens_the_vault_without_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut vault) = new_vault(&dir);
        let key = vault.export_key();
        let opened = Vault::open_with_key(&path, &key).unwrap();
        assert_eq!(opened.entries, vault.entries);
        // A vault opened by key saves a file the password still opens.
        vault.entries.clear();
        drop(vault);
        opened.save().unwrap();
        assert_eq!(
            Vault::open(&path, &password("master"))
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn foreign_key_is_reported_as_a_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = new_vault(&dir);
        assert!(matches!(
            Vault::open_with_key(&path, &[7u8; 32]),
            Err(VaultError::KeyMismatch)
        ));
    }

    #[test]
    fn id_is_readable_without_the_password_and_survives_saves() {
        let dir = tempfile::tempdir().unwrap();
        let (path, vault) = new_vault(&dir);
        assert_eq!(Vault::peek_id(&path).unwrap(), vault.id());
        vault.save().unwrap();
        assert_eq!(Vault::peek_id(&path).unwrap(), vault.id());
        let other = Vault::create(dir.path().join("other.lbx"), &password("master"), FAST).unwrap();
        assert_ne!(other.id(), vault.id());
    }

    #[cfg(unix)]
    #[test]
    fn vault_file_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = new_vault(&dir);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
