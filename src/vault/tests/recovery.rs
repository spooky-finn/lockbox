use crate::{
    access::mnemonic::RecoveryPhrase,
    vault::{witness::MemoryWitness, *},
};

const WORDS: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const FAST: KdfParams = KdfParams {
    memory_kib: 8192,
    passes: 1,
    lanes: 1,
};
fn password(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
fn configuration() -> SharedProfile {
    SharedProfile {
        username: "Alice".into(),
        space_name: "Team".into(),
        provider: ProviderConfiguration {
            base_url: "https://gateway.storjshare.io".into(),
            bucket: "test-bucket".into(),
            access_key: "fictional-access".into(),
            secret_key: "fictional-secret".into(),
        },
    }
}
fn store() -> ProfileStore {
    ProfileStore::new(Arc::new(MemoryWitness::default()))
}
fn create(store: &ProfileStore, path: &Path) -> Vault {
    store
        .create_shared_with_phrase(
            path,
            &password("original password"),
            FAST,
            configuration(),
            &RecoveryPhrase::parse(WORDS).unwrap(),
        )
        .unwrap()
}

#[test]
fn twelve_words_recover_records_and_reset_password_without_old_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let store = store();
    let phrase = RecoveryPhrase::parse(WORDS).unwrap();
    let mut vault = create(&store, &path);
    let mut entry = Entry::new();
    entry.title = "Recovery fixture".into();
    entry.password = "record secret".into();
    vault.entries.push(entry.clone());
    vault.save_changes().unwrap();
    let identity = vault.profile.as_ref().unwrap().identity.user_id;
    let device = vault.profile.as_ref().unwrap().identity.device_id;
    let key = vault.export_key();
    let id = vault.id();
    drop(vault);
    let data = fs::read(&path).unwrap();
    assert_eq!(u16::from_le_bytes(data[4..6].try_into().unwrap()), 8);
    assert!(!data.windows(WORDS.len()).any(|w| w == WORDS.as_bytes()));
    assert!(
        !data
            .windows(entry.password.len())
            .any(|w| w == entry.password.as_bytes())
    );
    let records = Vault::read_records(&path, &password("original password")).unwrap();
    assert_eq!(records.entries[0].password, entry.password);
    let recovered = store.open_with_recovery_phrase(&path, &phrase).unwrap();
    assert_eq!(recovered.entries[0].password, entry.password);
    let recovered = recovered
        .reset_password_with_phrase(&phrase, &password("new master password"))
        .unwrap();
    assert_eq!(recovered.id(), id);
    assert_eq!(*recovered.export_key(), *key);
    assert_eq!(
        recovered.profile.as_ref().unwrap().identity.user_id,
        identity
    );
    assert_eq!(
        recovered.profile.as_ref().unwrap().identity.device_id,
        device
    );
    assert!(recovered.signing_error().is_none());
    // The root capsule also accepts the new master password.
    recovered
        .profile
        .as_ref()
        .unwrap()
        .recover_identity(
            &password("new master password"),
            FAST,
            "Alice".into(),
            "Team".into(),
            configuration().provider,
        )
        .unwrap();
    drop(recovered);
    assert!(matches!(
        store.open(&path, &password("original password")),
        Err(VaultError::WrongPassword)
    ));
    let reopened = store.open(&path, &password("new master password")).unwrap();
    assert_eq!(reopened.entries[0].password, entry.password);
    drop(reopened);
    let biometric = store.open_with_key(&path, &key).unwrap();
    assert_eq!(biometric.entries[0].id, entry.id);
    drop(biometric);
    assert!(store.open_with_recovery_phrase(&path, &phrase).is_ok());
}

#[test]
fn wrong_words_tampering_and_truncated_slots_cannot_unlock_or_modify_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let store = store();
    drop(create(&store, &path));
    let original = fs::read(&path).unwrap();
    let phrase = RecoveryPhrase::parse(WORDS).unwrap();
    let wrong = RecoveryPhrase::parse(
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    )
    .unwrap();
    assert!(matches!(
        store.open_with_recovery_phrase(&path, &wrong),
        Err(VaultError::WrongRecoveryPhrase)
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
    for offset in [
        18,
        HEADER_LEN,
        HEADER_LEN + recovery::SLOT_LEN,
        HEADER_LEN + recovery::SLOTS_LEN + 1,
    ] {
        let mut damaged = original.clone();
        damaged[offset] ^= 1;
        fs::write(&path, damaged).unwrap();
        assert!(
            store.open_with_recovery_phrase(&path, &phrase).is_err(),
            "offset {offset}"
        );
        assert!(
            store.open(&path, &password("original password")).is_err(),
            "offset {offset}"
        );
    }
    for len in [5, HEADER_LEN, HEADER_LEN + recovery::SLOTS_LEN - 1] {
        fs::write(&path, &original[..len]).unwrap();
        assert!(store.open_with_recovery_phrase(&path, &phrase).is_err());
    }
    fs::write(&path, original).unwrap();
    assert!(store.open_with_recovery_phrase(&path, &phrase).is_ok());
}

#[test]
fn recovery_preserves_read_only_protection_on_a_different_installation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    drop(create(&store(), &path));
    let original = fs::read(&path).unwrap();
    let phrase = RecoveryPhrase::parse(WORDS).unwrap();
    let other = store();
    let recovered = other.open_with_recovery_phrase(&path, &phrase).unwrap();
    assert!(recovered.signing_error().is_some());
    assert!(
        recovered
            .reset_password_with_phrase(&phrase, &password("new master password"))
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn legacy_vaults_report_recovery_unavailable_and_keep_their_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.lbx");
    drop(Vault::create(&path, &password("old password"), FAST).unwrap());
    assert!(matches!(
        Vault::open_with_recovery_phrase(&path, &RecoveryPhrase::parse(WORDS).unwrap()),
        Err(VaultError::RecoveryUnavailable)
    ));
    assert!(Vault::open(&path, &password("old password")).is_ok());
}

#[test]
fn failed_password_reset_preserves_original_file_and_recovery_access() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let store = store();
    drop(create(&store, &path));
    let original = fs::read(&path).unwrap();
    let phrase = RecoveryPhrase::parse(WORDS).unwrap();
    let recovered = store.open_with_recovery_phrase(&path, &phrase).unwrap();
    let blocked = crate::vault::file::sibling(&path, "tmp");
    fs::create_dir(&blocked).unwrap();
    assert!(
        recovered
            .reset_password_with_phrase(&phrase, &password("new master password"))
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_dir(blocked).unwrap();
    drop(store.open(&path, &password("original password")).unwrap());
    assert!(store.open_with_recovery_phrase(&path, &phrase).is_ok());
}
