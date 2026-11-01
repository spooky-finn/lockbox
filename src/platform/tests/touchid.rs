use crate::{platform::touchid::*, sync::identity::Key};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// In-memory Touch ID and keychain.
pub(crate) struct FakeBiometrics {
    pub(crate) fingers: Mutex<Option<Vec<u8>>>,
    pub(crate) finger_matches: AtomicBool,
    pub(crate) prompts: AtomicUsize,
    pub(crate) items: Mutex<HashMap<String, Vec<u8>>>,
    pub(crate) store_error: Mutex<Option<String>>,
}

impl Default for FakeBiometrics {
    fn default() -> Self {
        Self {
            fingers: Mutex::new(Some(vec![1, 2, 3])),
            finger_matches: AtomicBool::new(true),
            prompts: AtomicUsize::new(0),
            items: Mutex::new(HashMap::new()),
            store_error: Mutex::new(None),
        }
    }
}

impl Biometrics for FakeBiometrics {
    fn domain_state(&self) -> Option<Vec<u8>> {
        self.fingers.lock().unwrap().clone()
    }
    fn authenticate(&self, _reason: &str) -> Result<Vec<u8>, String> {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        if !self.finger_matches.load(Ordering::SeqCst) {
            return Err("fingerprint not recognized".into());
        }
        self.domain_state().ok_or_else(|| "нет Touch ID".into())
    }
    fn load(&self, account: &str, reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        self.authenticate(reason)?;
        Ok(self
            .items
            .lock()
            .unwrap()
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }
    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        if let Some(error) = &*self.store_error.lock().unwrap() {
            return Err(error.clone());
        }
        self.items
            .lock()
            .unwrap()
            .insert(account.into(), data.to_vec());
        Ok(())
    }
    fn delete(&self, account: &str) {
        self.items.lock().unwrap().remove(account);
    }
}

const KEY: Key = [9; 32];
const DAY: u64 = 24 * 60 * 60;

fn enabled() -> FakeBiometrics {
    let fake = FakeBiometrics::default();
    enable(&fake, "vault", &KEY, 0).unwrap();
    fake
}

#[test]
fn enabled_vault_unlocks_with_the_finger() {
    let fake = enabled();
    assert_eq!(*unlock(&fake, "vault", DAY).unwrap(), KEY);
    assert_eq!(fake.prompts.load(Ordering::SeqCst), 2);
}

#[test]
fn enabling_requires_the_finger_first() {
    let fake = FakeBiometrics::default();
    fake.finger_matches.store(false, Ordering::SeqCst);
    assert!(matches!(
        enable(&fake, "vault", &KEY, 0),
        Err(TouchIdError::Failed(_))
    ));
    assert!(fake.items.lock().unwrap().is_empty());
}

#[test]
fn wrong_finger_gives_no_key_and_keeps_the_setting() {
    let fake = enabled();
    fake.finger_matches.store(false, Ordering::SeqCst);
    assert!(matches!(
        unlock(&fake, "vault", DAY),
        Err(TouchIdError::Failed(_))
    ));
    assert!(fake.items.lock().unwrap().contains_key("vault"));
}

#[test]
fn after_seven_days_the_password_is_required() {
    let fake = enabled();
    assert_eq!(
        unlock(&fake, "vault", 7 * DAY).unwrap_err(),
        TouchIdError::Expired
    );
    assert!(fake.items.lock().unwrap().is_empty());
}

#[test]
fn typing_the_password_restarts_the_seven_days() {
    let fake = enabled();
    refresh(&fake, "vault", &KEY, 6 * DAY).unwrap();
    assert!(unlock(&fake, "vault", 10 * DAY).is_ok());
}

#[test]
fn changed_finger_set_deletes_the_key() {
    let fake = enabled();
    *fake.fingers.lock().unwrap() = Some(vec![4, 5, 6]);
    assert_eq!(
        unlock(&fake, "vault", DAY).unwrap_err(),
        TouchIdError::FingersChanged
    );
    assert!(fake.items.lock().unwrap().is_empty());
}

#[test]
fn vault_without_a_saved_key_cannot_use_touch_id() {
    let fake = FakeBiometrics::default();
    assert_eq!(unlock(&fake, "vault", 0).unwrap_err(), TouchIdError::NoKey);
}

#[test]
fn disabling_deletes_the_key() {
    let fake = enabled();
    disable(&fake, "vault");
    assert!(fake.items.lock().unwrap().is_empty());
}

#[test]
fn computer_without_touch_id_is_reported() {
    let fake = FakeBiometrics::default();
    *fake.fingers.lock().unwrap() = None;
    assert!(!available(&fake));
    assert_eq!(
        unlock(&fake, "vault", 0).unwrap_err(),
        TouchIdError::Unavailable
    );
    assert_eq!(
        enable(&fake, "vault", &KEY, 0).unwrap_err(),
        TouchIdError::Unavailable
    );
}

#[test]
fn reading_storage_directly_requires_biometrics() {
    let fake = enabled();
    fake.finger_matches.store(false, Ordering::SeqCst);
    assert!(fake.load("vault", REASON).is_err());
}

#[test]
fn failed_secure_storage_does_not_enable_an_unprotected_key() {
    let fake = FakeBiometrics::default();
    *fake.store_error.lock().unwrap() = Some("missing Keychain entitlement".into());
    assert_eq!(
        enable(&fake, "vault", &KEY, 0),
        Err(TouchIdError::Keychain(
            "missing Keychain entitlement".into()
        ))
    );
    assert!(fake.items.lock().unwrap().is_empty());
    assert_eq!(
        unlock(&fake, "vault", DAY).unwrap_err(),
        TouchIdError::NoKey
    );
}

#[test]
fn invalid_stored_record_never_returns_a_key() {
    let fake = enabled();
    fake.items
        .lock()
        .unwrap()
        .insert("vault".into(), vec![0; 39]);
    assert_eq!(
        unlock(&fake, "vault", DAY).unwrap_err(),
        TouchIdError::NoKey
    );
}
