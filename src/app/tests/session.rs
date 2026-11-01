use super::*;
use crate::app::test_support::{security, unlocked};
use crate::{infra::config::Config, infra::touchid::tests::FakeBiometrics};

#[test]
fn locking_wipes_clipboard_discards_the_draft_and_keeps_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut workspace = unlocked(&dir);
    let path = workspace.vault.path().to_path_buf();
    workspace.start_new();
    workspace.draft.as_mut().unwrap().entry.title = "Unsaved".into();
    let mut screen = Screen::Unlocked(Box::new(workspace));
    let settings = Settings {
        config: Config::default(),
        path: None,
    };
    let mut native = security();
    native
        .clipboard
        .copy("secret", std::time::Instant::now())
        .unwrap();
    screen.lock(
        &settings,
        &FakeBiometrics::default(),
        &mut native,
        Some("timeout"),
    );
    assert!(
        native
            .clipboard
            .remaining(std::time::Instant::now())
            .is_none()
    );
    let Screen::Locked(form) = screen else {
        panic!("not locked");
    };
    assert_eq!(form.path.as_deref(), Some(path.as_path()));
    assert!(form.password.is_empty());
    assert!(form.notice.as_deref().unwrap().contains("timeout"));
    assert!(!form.auto_touch_id);
    let reopened =
        crate::infra::vault::Vault::open(&path, &secrecy::SecretString::from("master".to_owned()))
            .unwrap();
    assert!(reopened.entries().is_empty());
}

#[test]
fn opening_with_password_restores_biometric_key_and_remembers_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = unlocked(&dir);
    let path = workspace.vault.path().to_path_buf();
    let id = workspace.vault.id();
    let fake = FakeBiometrics::default();
    let mut settings = Settings {
        config: Config {
            touch_id: vec![id.clone()],
            ..Config::default()
        },
        path: None,
    };
    let mut native = security();
    let mut screen = Screen::Locked(Box::new(Locked::new(None)));
    screen.open(
        Opened {
            vault: workspace.vault,
            with_password: true,
        },
        &mut settings,
        &fake,
        &mut native,
    );
    assert!(fake.items.lock().unwrap().contains_key(&id));
    assert_eq!(settings.config.last_vault, Some(path));
    let Screen::Unlocked(workspace) = screen else {
        panic!("not open");
    };
    assert!(workspace.touch_id.enabled);
}

#[test]
fn joining_opens_sharing_and_survives_a_session_lock() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = unlocked(&dir);
    let mut settings = Settings {
        config: Config::default(),
        path: None,
    };
    let fake = FakeBiometrics::default();
    let mut native = security();
    let mut screen = Screen::Locked(Box::new(Locked::begin_join()));
    screen.open(
        Opened {
            vault: workspace.vault,
            with_password: true,
        },
        &mut settings,
        &fake,
        &mut native,
    );
    let Screen::Unlocked(workspace) = &screen else {
        panic!("not open")
    };
    assert!(workspace.page == crate::app::workspace::Page::Sharing);
    assert!(workspace.sharing.joining);
    screen.lock(&settings, &fake, &mut native, None);
    let Screen::Locked(form) = &screen else {
        panic!("not locked")
    };
    assert!(form.joining);
}
