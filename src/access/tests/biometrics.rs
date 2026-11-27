use crate::{
    access::biometrics::TouchId,
    desktop::test_support::unlocked,
    platform::touchid::{Biometrics, tests::FakeBiometrics},
    settings::{config::Config, state::Settings},
    vault::Vault,
};
use std::{
    sync::{Arc, Mutex, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

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

/// Holds the finger prompt open until the test releases it.
struct HeldPrompt {
    inner: FakeBiometrics,
    release: Mutex<mpsc::Receiver<()>>,
}

impl HeldPrompt {
    fn new() -> (Arc<Self>, mpsc::Sender<()>) {
        let (release, wait) = mpsc::channel();
        let held = Arc::new(Self {
            inner: FakeBiometrics::default(),
            release: Mutex::new(wait),
        });
        (held, release)
    }
}

impl Biometrics for HeldPrompt {
    fn domain_state(&self) -> Option<Vec<u8>> {
        self.inner.domain_state()
    }
    fn authenticate(&self, reason: &str) -> Result<Vec<u8>, String> {
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        self.inner.authenticate(reason)
    }
    fn load(&self, account: &str, reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        self.inner.load(account, reason)
    }
    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        self.inner.store(account, data)
    }
    fn delete(&self, account: &str) {
        self.inner.delete(account)
    }
}

/// The worker thread owns one clone; it is gone once the task has ended.
fn wait_for_worker_exit<T: ?Sized>(biometrics: &Arc<T>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Arc::strong_count(biometrics) > 1 {
        assert!(Instant::now() < deadline, "biometric worker timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_key(fake: &FakeBiometrics, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fake.items.lock().unwrap().contains_key(id) {
        assert!(Instant::now() < deadline, "biometric worker timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn saved_settings(directory: &tempfile::TempDir, touch_id: Vec<String>) -> Settings {
    let settings = Settings {
        config: Config {
            touch_id,
            ..Config::default()
        },
        path: Some(directory.path().join("config.json")),
    };
    settings.save().unwrap();
    settings
}

#[test]
fn locking_during_the_finger_prompt_never_stores_the_key() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let mut settings = saved_settings(&directory, Vec::new());
    let (held, release) = HeldPrompt::new();
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, held.clone(), true);
    assert!(control.pending.is_some());

    drop(control);
    release.send(()).unwrap();
    wait_for_worker_exit(&held);
    assert!(held.inner.items.lock().unwrap().is_empty());
}

#[test]
fn locking_after_the_key_was_stored_removes_the_untracked_key() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    let mut settings = saved_settings(&directory, Vec::new());
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, fake.clone(), true);
    wait_for_key(&fake, &id);

    drop(control);
    wait_for_worker_exit(&fake);
    assert!(fake.items.lock().unwrap().is_empty());
    assert!(
        Config::load(settings.path.as_deref().unwrap())
            .touch_id
            .is_empty()
    );
}

#[test]
fn locking_keeps_a_key_that_the_saved_config_already_references() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    // Another tab enabled Touch ID for this vault after this one opened.
    let mut settings = saved_settings(&directory, vec![id.clone()]);
    settings.config.touch_id.clear();
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, fake.clone(), true);
    wait_for_key(&fake, &id);

    drop(control);
    wait_for_worker_exit(&fake);
    assert!(fake.items.lock().unwrap().contains_key(&id));
}

#[test]
fn locking_keeps_the_key_when_the_saved_config_cannot_be_read() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    let mut settings = saved_settings(&directory, Vec::new());
    // A config this build cannot parse may still list the vault.
    std::fs::write(settings.path.as_deref().unwrap(), b"{\"touch_id\": 1}").unwrap();
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, fake.clone(), true);
    wait_for_key(&fake, &id);

    drop(control);
    wait_for_worker_exit(&fake);
    assert!(fake.items.lock().unwrap().contains_key(&id));
}

#[test]
fn a_delivered_enable_survives_the_lock() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let id = workspace.vault.id();
    let mut settings = saved_settings(&directory, Vec::new());
    let fake = Arc::new(FakeBiometrics::default());
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, fake.clone(), true);
    finish(&mut control, &workspace.vault, &mut settings).unwrap();

    drop(control);
    wait_for_worker_exit(&fake);
    assert!(fake.items.lock().unwrap().contains_key(&id));
}

#[test]
fn turning_off_during_a_pending_enable_is_not_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = unlocked(&directory);
    let mut settings = saved_settings(&directory, Vec::new());
    let (held, release) = HeldPrompt::new();
    let mut control = TouchId::default();
    control.set(&workspace.vault, &mut settings, held.clone(), true);

    assert!(
        control
            .set(&workspace.vault, &mut settings, held.clone(), false)
            .is_none()
    );
    assert!(control.pending.is_none());
    assert!(!control.enabled);
    release.send(()).unwrap();
    wait_for_worker_exit(&held);
    assert!(held.inner.items.lock().unwrap().is_empty());
    assert!(
        Config::load(settings.path.as_deref().unwrap())
            .touch_id
            .is_empty()
    );
}
