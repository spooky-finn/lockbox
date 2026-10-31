use super::{Locked, Mode, StorageKind, with_vault_extension};
use crate::app::test_support::unlocked;
use crate::infra::config::Config;
use crate::infra::touchid::tests::FakeBiometrics;
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
    assert!(locked.error.unwrap().contains("Файл не найден"));
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
fn cloud_or_unselected_creation_cannot_write_a_legacy_vault() {
    let dir = tempfile::tempdir().unwrap();
    for kind in [None, Some(StorageKind::Cloud)] {
        let path = dir.path().join("cloud.lbx");
        let mut form = Locked::new(Some(path.clone()));
        form.mode = Mode::Create;
        form.storage_kind = kind;
        form.password.push_str("a long master password");
        form.repeat.push_str("a long master password");
        form.submit();
        assert!(form.error.is_some());
        assert!(form.pending.is_none());
        assert!(!path.exists());
    }
}

#[test]
fn identity_creation_requires_distinct_confirmed_recovery_password() {
    let mut form = Locked::new(None);
    form.mode = Mode::Create;
    form.storage_kind = Some(StorageKind::Cloud);
    form.cloud_password_step = true;
    form.cloud.username = "Alice".into();
    form.cloud.space_name = "Team".into();
    form.cloud.base_url = "https://gateway.storjshare.io".into();
    form.cloud.bucket = "team-vault".into();
    form.cloud.access_key.push_str("test-access");
    form.cloud.secret_key.push_str("test-secret");
    form.password.push_str("local master password");
    form.repeat.push_str("local master password");
    assert!(!form.validate_creation());
    form.recovery_password.push_str("local master password");
    form.recovery_repeat.push_str("local master password");
    assert!(!form.validate_creation());
    form.recovery_password.clear();
    form.recovery_password
        .push_str("independent recovery password");
    assert!(!form.validate_creation());
    form.recovery_repeat.clear();
    form.recovery_repeat
        .push_str("independent recovery password");
    assert!(form.validate_creation());
    form.path = Some("/tmp/failed-profile.lbx".into());
    form.edit_cloud_connection();
    assert!(form.path.is_none());
    assert!(form.password.is_empty());
    assert!(form.recovery_password.is_empty());
    assert!(form.recovery_repeat.is_empty());
}

#[test]
fn application_worker_creates_and_reopens_identity_profile() {
    use crate::infra::vault::Vault;
    use secrecy::SecretString;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.lbx");
    let mut form = Locked::new(Some(path.clone()));
    form.mode = Mode::Create;
    form.storage_kind = Some(StorageKind::Cloud);
    form.cloud_password_step = true;
    form.cloud.username = "Alice".into();
    form.cloud.space_name = "Team".into();
    form.cloud.base_url = "https://gateway.storjshare.io".into();
    form.cloud.bucket = "team-vault".into();
    form.cloud.access_key.push_str("test-access");
    form.cloud.secret_key.push_str("test-secret");
    form.password.push_str("local master password");
    form.repeat.push_str("local master password");
    form.recovery_password
        .push_str("independent recovery password");
    form.recovery_repeat
        .push_str("independent recovery password");
    form.submit();
    assert!(form.error.is_none());
    assert!(form.password.is_empty());
    assert!(form.repeat.is_empty());
    assert!(form.recovery_password.is_empty());
    assert!(form.recovery_repeat.is_empty());
    let opened = form
        .pending
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let profile = opened.vault.profile.as_ref().unwrap();
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
    assert_eq!(reopened.profile.as_ref().unwrap().identity.user_id, id);
    assert_eq!(
        reopened.profile.as_ref().unwrap().identity.device_id,
        device
    );
}
