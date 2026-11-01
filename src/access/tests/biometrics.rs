use crate::{
    access::biometrics::TouchId,
    desktop::test_support::unlocked,
    platform::touchid::tests::FakeBiometrics,
    settings::{config::Config, state::Settings},
    vault::Vault,
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

fn finish(control: &mut TouchId, vault: &Vault, settings: &mut Settings) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = control.poll(vault, settings) {
            return result;
        }
        assert!(Instant::now() < deadline, "biometric worker timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn enabling_saves_preferences_after_poll_and_disabling_removes_the_key() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    let path = directory.path().join("config.json");
    let mut settings = Settings {
        config: Config::default(),
        path: Some(path.clone()),
    };
    settings.save().unwrap();
    let before = std::fs::read(&path).unwrap();
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    assert!(
        control
            .set(&workspace.vault, &mut settings, fake.clone(), true)
            .is_none()
    );
    assert!(control.pending.is_some());
    assert!(!control.enabled);
    assert!(settings.config.touch_id.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    finish(&mut control, &workspace.vault, &mut settings).unwrap();
    assert!(control.pending.is_none());
    assert!(control.enabled);
    assert!(fake.items.lock().unwrap().contains_key(&id));
    assert_eq!(
        Config::load(&path).touch_id.as_slice(),
        std::slice::from_ref(&id)
    );
    assert!(
        control
            .set(&workspace.vault, &mut settings, fake.clone(), false)
            .is_none()
    );
    assert!(!control.enabled);
    assert!(control.pending.is_none());
    assert!(!fake.items.lock().unwrap().contains_key(&id));
    assert!(Config::load(&path).touch_id.is_empty());
}

#[test]
fn native_enable_failures_keep_preferences_and_control_disabled() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let path = directory.path().join("config.json");
    let mut settings = Settings {
        config: Config::default(),
        path: Some(path.clone()),
    };
    settings.save().unwrap();
    let before = std::fs::read(&path).unwrap();
    for denied in [true, false] {
        let fake = Arc::new(FakeBiometrics::default());
        let expected = if denied {
            fake.finger_matches.store(false, Ordering::SeqCst);
            "Could not enable Touch ID: Touch ID: fingerprint not recognized"
        } else {
            *fake.store_error.lock().unwrap() = Some("fixture unavailable".into());
            "Could not enable Touch ID: keychain: fixture unavailable"
        };
        let mut control = TouchId::default();
        control.set(&workspace.vault, &mut settings, fake.clone(), true);
        assert_eq!(
            finish(&mut control, &workspace.vault, &mut settings),
            Err(expected.into())
        );
        assert!(control.pending.is_none());
        assert!(!control.enabled);
        assert!(fake.items.lock().unwrap().is_empty());
        assert!(settings.config.touch_id.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn preference_write_failure_keeps_the_successfully_enabled_native_key() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    let blocked = directory.path().join("config.json");
    std::fs::create_dir(&blocked).unwrap();
    let mut settings = Settings {
        config: Config::default(),
        path: Some(blocked),
    };
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, fake.clone(), true);
    let error = finish(&mut control, &workspace.vault, &mut settings).unwrap_err();
    assert!(error.starts_with("Could not save settings:"));
    assert!(control.pending.is_none());
    assert!(control.enabled);
    assert!(fake.items.lock().unwrap().contains_key(&id));
    assert_eq!(settings.config.touch_id, [id]);
}
