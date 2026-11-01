//! Data protection Keychain adapter; never reads legacy unlock secrets.

use std::sync::mpsc;

use block2::RcBlock;
use core_foundation::{
    base::{CFType, TCFType},
    string::CFString,
};
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework::{
    access_control::{ProtectionMode, SecAccessControl},
    passwords::{
        AccessControlOptions, PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    },
};
use security_framework_sys::{
    base::errSecItemNotFound,
    item::{kSecAttrAccount, kSecUseAuthenticationContext},
};
use zeroize::Zeroizing;

use crate::platform::touchid::Biometrics;

const SERVICE: &str = "lockbox.touch-id.v2";
const LEGACY_SERVICE: &str = "lockbox";
const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

pub struct MacBiometrics;

#[allow(deprecated)] // `domainState` replaces it only from macOS 15.
fn current_domain_state(context: &LAContext) -> Option<Vec<u8>> {
    unsafe { context.evaluatedPolicyDomainState() }.map(|data| data.to_vec())
}

fn describe(error: &NSError) -> String {
    // LAError codes: userCancel, systemCancel, appCancel, biometryLockout.
    match error.code() {
        -2 | -4 | -9 => "cancelled".to_owned(),
        -1 => "fingerprint not recognized".to_owned(),
        -8 => "Touch ID is locked after failed attempts — enter the master password".to_owned(),
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
                    .unwrap_or_else(|| "unknown error".to_owned()))
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
            .map_err(|_| "Touch ID did not respond".to_owned())??;
        current_domain_state(&context).ok_or_else(|| "no fingerprint data".to_owned())
    }

    fn load(&self, account: &str, reason: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        purge_legacy()?;
        // SecItemCopyMatching displays its own authentication prompt. A separate
        // evaluatePolicy success must never authorize an unprotected key read.
        let context = unsafe { LAContext::new() };
        unsafe { context.setLocalizedReason(&NSString::from_str(reason)) };
        match generic_password(read_options(account, &context)) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(error) if error.code() == errSecItemNotFound => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        purge_legacy()?;
        let options = storage_options(account)?;
        // Recreate rather than update: every saved item must receive the ACL.
        delete_protected(account)?;
        set_generic_password_options(data, options).map_err(keychain_error)
    }

    fn delete(&self, account: &str) {
        let _ = purge_legacy();
        let _ = delete_protected(account);
    }
}

impl MacBiometrics {
    pub(super) fn new() -> Self {
        // Retire *all* old unlock keys, including vaults not opened this run.
        // Load/store retry cleanup and surface failures before touching keys.
        let _ = purge_legacy();
        Self
    }
}

#[allow(deprecated)] // PasswordOptions has no authentication-context setter.
fn read_options(account: &str, context: &LAContext) -> PasswordOptions {
    let mut options = protected_options(account);
    // Security accepts an LAContext object for this key. Retain it in the query
    // through SecItemCopyMatching; each read uses a fresh, unevaluated context.
    unsafe {
        options.query.push((
            CFString::wrap_under_get_rule(kSecUseAuthenticationContext),
            CFType::wrap_under_get_rule((context as *const LAContext).cast()),
        ));
    }
    options
}

fn protected_options(account: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(SERVICE, account);
    options.use_protected_keychain();
    options.set_access_synchronized(Some(false));
    options
}

fn storage_options(account: &str) -> Result<PasswordOptions, String> {
    let access = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
    )
    .map_err(keychain_error)?;
    let mut options = protected_options(account);
    options.set_access_control(access);
    Ok(options)
}

#[allow(deprecated)] // PasswordOptions has no service-only deletion builder.
fn legacy_options() -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(LEGACY_SERVICE, "");
    let account = unsafe { CFString::wrap_under_get_rule(kSecAttrAccount) };
    options.query.retain(|(key, _)| key != &account);
    options
}

fn purge_legacy() -> Result<(), String> {
    delete_if_present(legacy_options())
        .map_err(|error| format!("Could not remove legacy Touch ID keys: {error}"))
}

fn delete_protected(account: &str) -> Result<(), String> {
    delete_if_present(protected_options(account))
}

fn delete_if_present(options: PasswordOptions) -> Result<(), String> {
    match delete_generic_password_options(options) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == errSecItemNotFound => Ok(()),
        Err(error) => Err(keychain_error(error)),
    }
}

fn keychain_error(error: security_framework::base::Error) -> String {
    if error.code() == -34018 {
        "Secure Touch ID storage requires a signed app with Keychain access entitlements; use the master password".into()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
#[allow(deprecated)] // Inspect the dictionaries passed to Security.framework.
mod tests {
    use super::*;
    use core_foundation::boolean::CFBoolean;
    use security_framework_sys::item::{
        kSecAttrAccessControl, kSecAttrService, kSecAttrSynchronizable,
        kSecUseDataProtectionKeychain,
    };

    #[test]
    fn legacy_cleanup_covers_all_accounts_in_only_the_old_service() {
        let options = legacy_options();
        let service = unsafe { CFString::wrap_under_get_rule(kSecAttrService) };
        let account = unsafe { CFString::wrap_under_get_rule(kSecAttrAccount) };
        assert!(options.query.iter().any(|(key, value)| {
            key == &service && value == &CFString::from(LEGACY_SERVICE).into_CFType()
        }));
        assert!(!options.query.iter().any(|(key, _)| key == &account));
        assert_ne!(LEGACY_SERVICE, SERVICE);
    }

    #[test]
    fn protected_storage_uses_local_data_protection_and_access_control() {
        let options = storage_options("test-account").unwrap();
        for (key, expected) in [
            (unsafe { kSecUseDataProtectionKeychain }, true),
            (unsafe { kSecAttrSynchronizable }, false),
        ] {
            let key = unsafe { CFString::wrap_under_get_rule(key) };
            assert!(options.query.iter().any(|(actual, value)| {
                actual == &key && value == &CFBoolean::from(expected).into_CFType()
            }));
        }
        let access = unsafe { CFString::wrap_under_get_rule(kSecAttrAccessControl) };
        assert!(options.query.iter().any(|(key, _)| key == &access));
    }

    #[test]
    #[ignore = "OS probe: requires signed Keychain entitlements and enrolled Touch ID; creates one random item"]
    fn keychain_refuses_to_release_key_without_biometric_interaction() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let account = format!("touch-id-probe-{:x?}", random);
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete_protected(&self.0);
            }
        }
        let _cleanup = Cleanup(account.clone());
        set_generic_password_options(&[7; 32], storage_options(&account).unwrap()).unwrap();
        let context = unsafe { LAContext::new() };
        unsafe { context.setInteractionNotAllowed(true) };
        // This is an actual SecItemCopyMatching read, with no prior evaluatePolicy.
        // An entitlement error or a missing item must not count as proof of gating.
        let error = generic_password(read_options(&account, &context)).unwrap_err();
        assert!(matches!(error.code(), -25308 | -25293), "{error}");
    }
}
