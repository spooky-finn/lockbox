use super::{Locked, with_vault_extension};
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
