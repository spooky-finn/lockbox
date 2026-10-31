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
