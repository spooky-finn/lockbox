//! Touch ID unlock, simplified variant.
//!
//! The vault key lives in the login keychain and lockbox itself asks
//! LocalAuthentication to check the finger. macOS does not bind the keychain
//! item to the finger: a program the user allows to read the `lockbox` item
//! gets the key without Touch ID. Binding it needs a signing identity from an
//! Apple Developer account (`errSecMissingEntitlement` without one).

use std::sync::Arc;

use zeroize::Zeroizing;

/// The master password is required again this long after it was last typed.
pub const PASSWORD_EVERY_SECS: u64 = 7 * 24 * 60 * 60;

const REASON: &str = "разблокировать lockbox";

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TouchIdError {
    #[error("Touch ID недоступен на этом компьютере")]
    Unavailable,
    #[error("Touch ID: {0}")]
    Failed(String),
    #[error("Touch ID для этого хранилища не включён")]
    NoKey,
    #[error("прошло 7 дней с последнего ввода мастер-пароля — введите его")]
    Expired,
    #[error("набор отпечатков в системе изменился — введите мастер-пароль")]
    FingersChanged,
    #[error("связка ключей: {0}")]
    Keychain(String),
}

/// Touch ID and keychain access; tests substitute an in-memory fake.
pub trait Biometrics: Send + Sync {
    /// Opaque value that changes when fingers are added or removed;
    /// `None` when Touch ID is unavailable.
    fn domain_state(&self) -> Option<Vec<u8>>;
    /// Shows the Touch ID prompt; on success returns the current `domain_state`.
    fn authenticate(&self, reason: &str) -> Result<Vec<u8>, String>;
    fn load(&self, account: &str) -> Option<Zeroizing<Vec<u8>>>;
    fn store(&self, account: &str, data: &[u8]) -> Result<(), String>;
    fn delete(&self, account: &str);
}

/// Keychain item: key, when the password was last typed, and the finger set.
struct Record {
    key: Zeroizing<[u8; 32]>,
    saved_at: u64,
    domain_state: Vec<u8>,
}

impl Record {
    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(40 + self.domain_state.len()));
        bytes.extend_from_slice(self.key.as_ref());
        bytes.extend_from_slice(&self.saved_at.to_le_bytes());
        bytes.extend_from_slice(&self.domain_state);
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            return None;
        }
        Some(Self {
            key: Zeroizing::new(bytes[..32].try_into().ok()?),
            saved_at: u64::from_le_bytes(bytes[32..40].try_into().ok()?),
            domain_state: bytes[40..].to_vec(),
        })
    }
}

pub fn available(biometrics: &dyn Biometrics) -> bool {
    biometrics.domain_state().is_some()
}

/// Asks for the finger, then returns the saved key. A key that expired or was
/// saved for a different finger set is deleted, so only the password opens.
pub fn unlock(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    now: u64,
) -> Result<Zeroizing<[u8; 32]>, TouchIdError> {
    if !available(biometrics) {
        return Err(TouchIdError::Unavailable);
    }
    let fingers = biometrics
        .authenticate(REASON)
        .map_err(TouchIdError::Failed)?;
    let record = biometrics
        .load(vault_id)
        .and_then(|bytes| Record::decode(&bytes))
        .ok_or(TouchIdError::NoKey)?;
    if record.domain_state != fingers {
        biometrics.delete(vault_id);
        return Err(TouchIdError::FingersChanged);
    }
    if now.saturating_sub(record.saved_at) >= PASSWORD_EVERY_SECS {
        biometrics.delete(vault_id);
        return Err(TouchIdError::Expired);
    }
    Ok(record.key)
}

/// Turns Touch ID on: the finger is checked before the key is stored.
pub fn enable(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &[u8; 32],
    now: u64,
) -> Result<(), TouchIdError> {
    if !available(biometrics) {
        return Err(TouchIdError::Unavailable);
    }
    let fingers = biometrics
        .authenticate(REASON)
        .map_err(TouchIdError::Failed)?;
    save(biometrics, vault_id, key, now, fingers)
}

/// After the master password was typed: stores a fresh key, restarting the 7 days.
pub fn refresh(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &[u8; 32],
    now: u64,
) -> Result<(), TouchIdError> {
    let fingers = biometrics.domain_state().ok_or(TouchIdError::Unavailable)?;
    save(biometrics, vault_id, key, now, fingers)
}

pub fn disable(biometrics: &dyn Biometrics, vault_id: &str) {
    biometrics.delete(vault_id);
}

fn save(
    biometrics: &dyn Biometrics,
    vault_id: &str,
    key: &[u8; 32],
    now: u64,
    domain_state: Vec<u8>,
) -> Result<(), TouchIdError> {
    let record = Record {
        key: Zeroizing::new(*key),
        saved_at: now,
        domain_state,
    };
    biometrics
        .store(vault_id, &record.encode())
        .map_err(TouchIdError::Keychain)
}

/// Touch ID of this computer, or a stand-in that reports it as unavailable.
pub fn system() -> Arc<dyn Biometrics> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(mac::MacBiometrics)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(Unsupported)
    }
}

#[cfg(not(target_os = "macos"))]
struct Unsupported;

#[cfg(not(target_os = "macos"))]
impl Biometrics for Unsupported {
    fn domain_state(&self) -> Option<Vec<u8>> {
        None
    }
    fn authenticate(&self, _reason: &str) -> Result<Vec<u8>, String> {
        Err("не поддерживается".into())
    }
    fn load(&self, _account: &str) -> Option<Zeroizing<Vec<u8>>> {
        None
    }
    fn store(&self, _account: &str, _data: &[u8]) -> Result<(), String> {
        Err("не поддерживается".into())
    }
    fn delete(&self, _account: &str) {}
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use zeroize::Zeroizing;

    use super::Biometrics;

    const SERVICE: &str = "lockbox";
    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

    pub struct MacBiometrics;

    #[allow(deprecated)] // `domainState` replaces it only from macOS 15.
    fn current_domain_state(context: &LAContext) -> Option<Vec<u8>> {
        unsafe { context.evaluatedPolicyDomainState() }.map(|data| data.to_vec())
    }

    fn describe(error: &NSError) -> String {
        // LAError codes: userCancel, systemCancel, appCancel, biometryLockout.
        match error.code() {
            -2 | -4 | -9 => "отменено".to_owned(),
            -1 => "палец не распознан".to_owned(),
            -8 => {
                "Touch ID заблокирован после неудачных попыток — введите мастер-пароль".to_owned()
            }
            _ => error.localizedDescription().to_string(),
        }
    }

    impl Biometrics for MacBiometrics {
        fn domain_state(&self) -> Option<Vec<u8>> {
            let context = unsafe { LAContext::new() };
            unsafe { context.canEvaluatePolicy_error(POLICY) }.ok()?;
            current_domain_state(&context)
        }

        fn authenticate(&self, reason: &str) -> Result<Vec<u8>, String> {
            let context = unsafe { LAContext::new() };
            let (sender, receiver) = mpsc::channel();
            let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
                let result = if ok.as_bool() {
                    Ok(())
                } else {
                    Err(unsafe { error.as_ref() }
                        .map(describe)
                        .unwrap_or_else(|| "неизвестная ошибка".to_owned()))
                };
                let _ = sender.send(result);
            });
            unsafe {
                context.evaluatePolicy_localizedReason_reply(
                    POLICY,
                    &NSString::from_str(reason),
                    &reply,
                )
            };
            receiver
                .recv()
                .map_err(|_| "Touch ID не ответил".to_owned())??;
            current_domain_state(&context).ok_or_else(|| "нет данных об отпечатках".to_owned())
        }

        fn load(&self, account: &str) -> Option<Zeroizing<Vec<u8>>> {
            get_generic_password(SERVICE, account)
                .ok()
                .map(Zeroizing::new)
        }

        fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
            set_generic_password(SERVICE, account, data).map_err(|error| error.to_string())
        }

        fn delete(&self, account: &str) {
            let _ = delete_generic_password(SERVICE, account);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
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
    }

    impl Default for FakeBiometrics {
        fn default() -> Self {
            Self {
                fingers: Mutex::new(Some(vec![1, 2, 3])),
                finger_matches: AtomicBool::new(true),
                prompts: AtomicUsize::new(0),
                items: Mutex::new(HashMap::new()),
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
                return Err("палец не распознан".into());
            }
            self.domain_state().ok_or_else(|| "нет Touch ID".into())
        }
        fn load(&self, account: &str) -> Option<Zeroizing<Vec<u8>>> {
            self.items
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(Zeroizing::new)
        }
        fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
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

    const KEY: [u8; 32] = [9; 32];
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
}
