use crate::{
    access::auth::{CreationStep, Locked, Mode, StorageKind, with_vault_extension},
    desktop::test_support::unlocked,
    platform::touchid::tests::FakeBiometrics,
    settings::config::Config,
};
use std::path::PathBuf;

#[test]
fn touch_id_is_offered_only_where_it_was_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    let unlocked = unlocked(&dir);
    let path = unlocked.vault.path().to_path_buf();
    let mut config = Config::default();
    let fake = FakeBiometrics::default();

    let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
    assert!(locked.touch_id.is_none());
    assert!(!locked.auto_touch_id);

    config.touch_id.push(unlocked.vault.id());
    let locked = Locked::new(Some(path.clone())).offer_touch_id(&config, &fake, true);
    assert_eq!(locked.touch_id, Some(unlocked.vault.id()));
    assert!(locked.auto_touch_id);

    *fake.fingers.lock().unwrap() = None;
    let locked = Locked::new(Some(path)).offer_touch_id(&config, &fake, true);
    assert!(locked.touch_id.is_none(), "no sensor, no Touch ID button");
}

#[test]
fn remembered_file_that_disappeared_is_reported() {
    let locked = Locked::new(Some("/nonexistent/vault.lbx".into()));
    assert!(locked.error.unwrap().contains("File not found"));
}

#[test]
fn picked_files_get_the_vault_extension() {
    assert_eq!(
        with_vault_extension("/tmp/secrets".into()),
        PathBuf::from("/tmp/secrets.lbx")
    );
    assert_eq!(
        with_vault_extension("/tmp/vault.lbx".into()),
        PathBuf::from("/tmp/vault.lbx")
    );
}

#[test]
fn creation_cannot_skip_key_setup_or_sync_selection() {
    let dir = tempfile::tempdir().unwrap();
    for kind in [None, Some(StorageKind::Cloud), Some(StorageKind::Local)] {
        let path = dir.path().join("cloud.lbx");
        let mut form = Locked::new(Some(path.clone()));
        form.mode = Mode::Create;
        form.storage_kind = kind;
        form.password.push_str("a long master password");
        form.repeat.push_str("a long master password");
        form.submit();
        assert!(form.pending.is_none());
        assert!(!path.exists());
    }
}

#[test]
fn key_setup_requires_valid_saved_words_and_preserves_them_across_sync_choices() {
    let dir = tempfile::tempdir().unwrap();
    let mut form = Locked::begin_create();
    let generated = zeroize::Zeroizing::new(form.recovery_words.to_string());
    assert!(crate::access::mnemonic::RecoveryPhrase::parse(&generated).is_ok());
    form.choose_local_path(dir.path().join("skipped"));
    assert!(form.path.is_none());
    form.continue_key_setup();
    assert!(form.creation_step == CreationStep::EncryptionKey);
    assert!(
        form.error
            .as_ref()
            .unwrap()
            .contains("Save your recovery words")
    );

    form.recovery_words.clear();
    form.recovery_words.push_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon");
    form.words_saved = true;
    form.continue_key_setup();
    assert!(form.creation_step == CreationStep::EncryptionKey);
    assert!(form.error.as_ref().unwrap().contains("12-word"));

    form.recovery_words.clear();
    form.recovery_words.push_str(&generated);
    form.show_recovery_words = true;
    form.continue_key_setup();
    assert!(form.creation_step == CreationStep::Sync);
    assert!(!form.show_recovery_words);
    form.choose_local_path(dir.path().join("chosen"));
    assert!(form.creation_step == CreationStep::Password);
    assert_eq!(form.path, Some(dir.path().join("chosen.lbx")));
    assert_eq!(form.recovery_words.as_str(), generated.as_str());
    form.password.push_str("local master password");
    form.repeat.push_str("local master password");
    assert!(form.validate_creation());
    form.edit_sync_setup();
    assert!(form.path.is_none());
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    form.storage_kind = Some(StorageKind::Cloud);
    form.cloud.username = "Alice".into();
    form.cloud.space_name = "Team".into();
    form.cloud.base_url = "https://gateway.storjshare.io".into();
    form.cloud.bucket = "team-vault".into();
    form.cloud.access_key.push_str("test-access");
    form.cloud.secret_key.push_str("test-secret");
    form.continue_cloud();
    assert!(form.creation_step == CreationStep::Password);
    assert_eq!(form.recovery_words.as_str(), generated.as_str());
    assert!(form.words_saved);
    form.edit_recovery_key();
    assert!(form.creation_step == CreationStep::EncryptionKey);
    assert!(form.generate_recovery_words());
    assert!(!form.words_saved);
    form.continue_key_setup();
    assert!(form.creation_step == CreationStep::EncryptionKey);
}

#[test]
fn application_worker_creates_local_vault_with_the_confirmed_key() {
    use crate::{access::mnemonic::RecoveryPhrase, vault::Vault};
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("local.lbx");
    let mut form = Locked::begin_create();
    let phrase = RecoveryPhrase::parse(&form.recovery_words).unwrap();
    form.words_saved = true;
    form.continue_key_setup();
    form.choose_local_path(path.clone());
    form.password.push_str("local master password");
    form.repeat.push_str("local master password");
    form.words_saved = false;
    form.submit();
    assert!(form.pending.is_none());
    assert!(!path.exists());
    form.words_saved = true;
    form.submit();
    assert!(form.error.is_none());
    let opened = form
        .pending
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert!(opened.vault.test_profile().is_none());
    drop(opened);
    let recovered = Vault::open_with_recovery_phrase(&path, &phrase).unwrap();
    assert!(recovered.test_profile().is_none());
}

#[test]
fn identity_creation_requires_confirmed_password_and_saved_valid_recovery_words() {
    let mut form = Locked::begin_create();
    form.words_saved = true;
    form.continue_key_setup();
    form.storage_kind = Some(StorageKind::Cloud);
    form.cloud.username = "Alice".into();
    form.cloud.space_name = "Team".into();
    form.cloud.base_url = "https://gateway.storjshare.io".into();
    form.cloud.bucket = "team-vault".into();
    form.cloud.access_key.push_str("test-access");
    form.cloud.secret_key.push_str("test-secret");
    form.continue_cloud();
    assert!(form.creation_step == CreationStep::Password);
    form.words_saved = false;
    form.password.push_str("too short");
    form.repeat.push_str("too short");
    assert!(!form.validate_creation());
    form.password.clear();
    form.password.push_str("local master password");
    assert!(!form.validate_creation());
    form.repeat.clear();
    form.repeat.push_str("local master password");
    assert!(!form.validate_creation());
    form.words_saved = true;
    assert!(form.validate_creation());
    let valid_words = form.recovery_words.to_string();
    form.recovery_words.clear();
    form.recovery_words.push_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon");
    assert!(!form.validate_creation());
    form.recovery_words.clear();
    form.recovery_words.push_str(&valid_words);
    assert!(form.validate_creation());
    form.path = Some("/tmp/failed-profile.lbx".into());
    form.edit_sync_setup();
    assert!(form.path.is_none());
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    assert_eq!(form.recovery_words.as_str(), valid_words);
    assert!(form.words_saved);
    assert!(form.creation_step == CreationStep::Sync);
}

#[test]
fn application_worker_creates_and_reopens_identity_profile() {
    use crate::vault::Vault;
    use secrecy::SecretString;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let mut form = Locked::begin_create();
    form.words_saved = true;
    form.continue_key_setup();
    form.storage_kind = Some(StorageKind::Cloud);
    form.cloud.username = "Alice".into();
    form.cloud.space_name = "Team".into();
    form.cloud.base_url = "https://gateway.storjshare.io".into();
    form.cloud.bucket = "team-vault".into();
    form.cloud.access_key.push_str("test-access");
    form.cloud.secret_key.push_str("test-secret");
    form.continue_cloud();
    form.path = Some(path.clone());
    form.password.push_str("local master password");
    form.repeat.push_str("local master password");
    form.words_saved = true;
    let words = zeroize::Zeroizing::new(form.recovery_words.to_string());
    let expected_root = crate::access::mnemonic::RecoveryPhrase::parse(&form.recovery_words)
        .unwrap()
        .signing_key()
        .verifying_key()
        .to_bytes();
    form.submit();
    assert!(form.error.is_none());
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    let opened = form
        .pending
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let profile = opened.vault.test_profile().unwrap();
    assert_eq!(profile.identity.root_key, expected_root);
    assert_eq!(profile.username, "Alice");
    assert_eq!(profile.space_name, "Team");
    let id = profile.identity.user_id;
    let device = profile.identity.device_id;
    drop(opened);
    let reopened = Vault::open(
        &path,
        &SecretString::from("local master password".to_owned()),
    )
    .unwrap();
    assert_eq!(reopened.test_profile().unwrap().identity.user_id, id);
    assert_eq!(reopened.test_profile().unwrap().identity.device_id, device);
    let profile = reopened.test_profile().unwrap();
    let recovered = profile
        .recover_identity(
            &SecretString::from("local master password".to_owned()),
            crate::vault::KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            profile.username.clone(),
            profile.space_name.clone(),
            crate::vault::profile::ProviderConfiguration {
                base_url: profile.provider.base_url.clone(),
                bucket: profile.provider.bucket.clone(),
                access_key: profile.provider.access_key.clone(),
                secret_key: profile.provider.secret_key.clone(),
            },
        )
        .unwrap();
    assert_eq!(recovered.identity.user_id, id);
    assert_ne!(recovered.identity.device_id, device);
    drop(reopened);
    let mut recovery = Locked::new(Some(path.clone()));
    recovery.toggle_recovery();
    recovery.password.push_str("replacement master password");
    recovery.repeat.push_str("replacement master password");
    recovery.recovery_words.push_str(&words);
    recovery.submit();
    assert!(recovery.error.is_none());
    let recovered = recovery
        .pending
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert!(recovered.with_password);
    assert!(recovered.recovery_notice.is_none());
    assert_eq!(recovered.vault.test_profile().unwrap().identity.user_id, id);
    drop(recovered);
    assert!(
        Vault::open(
            &path,
            &SecretString::from("local master password".to_owned())
        )
        .is_err()
    );
    assert!(
        Vault::open(
            &path,
            &SecretString::from("replacement master password".to_owned())
        )
        .is_ok()
    );
}

#[test]
fn recovery_mode_clears_secrets_and_validates_before_starting_worker() {
    let mut form = Locked::new(Some("/tmp/missing-recovery-test.lbx".into()));
    form.password.push_str("old typed secret");
    form.toggle_recovery();
    assert!(form.recovering);
    assert!(form.password.is_empty());
    form.password.push_str("new master password");
    form.repeat.push_str("new master password");
    form.recovery_words.push_str("invalid recovery words");
    form.submit();
    assert!(form.pending.is_none());
    assert!(form.error.as_ref().unwrap().contains("12-word"));
    form.toggle_recovery();
    assert!(!form.recovering);
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    assert!(form.recovery_words.is_empty());
}

fn wait_for_identity(form: &mut Locked) {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while form.busy() && Instant::now() < deadline {
        assert!(form.poll(Arc::new(FakeBiometrics::default())).is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!form.busy(), "identity worker timed out");
}

#[test]
fn identity_is_saved_without_a_space_and_reused_after_restart_for_joining() {
    use crate::{access::mnemonic::RecoveryPhrase, vault::identity::Identity};
    use secrecy::SecretString;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.key");
    let mut form = Locked::begin_create().with_identity_path(Some(path.clone()), None);
    let phrase = RecoveryPhrase::parse(&form.recovery_words).unwrap();
    let root = phrase.signing_key().to_bytes();
    form.words_saved = true;
    form.continue_key_setup();
    assert!(form.creation_step == CreationStep::Identity);
    form.password.push_str("identity master password");
    form.repeat.push_str("identity master password");
    form.submit();
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    wait_for_identity(&mut form);
    assert!(form.error.is_none());
    assert!(form.creation_step == CreationStep::Sync);
    assert!(form.path.is_none());
    assert!(form.storage_kind.is_none());
    assert!(path.exists());
    assert!(!form.generate_recovery_words());
    form.edit_recovery_key();
    assert!(form.creation_step == CreationStep::Sync);
    form.switch_creation(true);
    assert!(form.joining);
    assert_eq!(
        form.identity.as_ref().unwrap().signing_key().to_bytes(),
        root
    );
    drop(form);

    let bytes = std::fs::read(&path).unwrap();
    let mut join = Locked::begin_join().with_identity_path(Some(path.clone()), None);
    assert!(join.creation_step == CreationStep::Identity);
    assert!(join.recovery_words.is_empty());
    assert!(!join.generate_recovery_words());
    join.password.push_str("wrong password");
    join.submit();
    wait_for_identity(&mut join);
    assert!(join.identity.is_none());
    assert!(join.creation_step == CreationStep::Identity);
    assert!(join.error.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    join.password.push_str("identity master password");
    join.submit();
    wait_for_identity(&mut join);
    assert!(join.error.is_none());
    assert!(join.creation_step == CreationStep::Sync);
    assert_eq!(
        join.identity.as_ref().unwrap().signing_key().to_bytes(),
        root
    );
    assert_eq!(
        Identity::open(
            &path,
            &SecretString::from("identity master password".to_owned())
        )
        .unwrap()
        .signing_key()
        .to_bytes(),
        root
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn joining_adopts_the_existing_profile_root_without_creating_another() {
    use crate::vault::{KdfParams, ProfileStore, ProviderConfiguration, SharedProfile};
    use secrecy::SecretString;
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("existing.lbx");
    let password = SecretString::from("existing master password".to_owned());
    let vault = ProfileStore::default()
        .create_shared(
            &source,
            &password,
            &password,
            KdfParams {
                memory_kib: 8192,
                passes: 1,
                lanes: 1,
            },
            SharedProfile {
                username: "Alice".into(),
                space_name: "Existing".into(),
                provider: ProviderConfiguration {
                    base_url: "https://gateway.storjshare.io".into(),
                    bucket: "fictional-bucket".into(),
                    access_key: "fictional-access".into(),
                    secret_key: "fictional-secret".into(),
                },
            },
        )
        .unwrap();
    let root = vault.test_profile().unwrap().identity.root_key;
    let before = std::fs::read(&source).unwrap();
    drop(vault);
    let mut form = Locked::begin_join().with_identity_path(
        Some(directory.path().join("identity.key")),
        Some(source.clone()),
    );
    assert!(form.unlocking_identity());
    assert!(form.recovery_words.is_empty());
    form.password.push_str("existing master password");
    form.submit();
    wait_for_identity(&mut form);
    assert!(form.error.is_none(), "{:?}", form.error);
    assert_eq!(
        form.identity
            .as_ref()
            .unwrap()
            .signing_key()
            .verifying_key()
            .to_bytes(),
        root
    );
    assert_eq!(std::fs::read(&source).unwrap(), before);
}
