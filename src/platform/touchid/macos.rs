//! Secure Enclave biometric encryption; Keychain stores only sealed envelopes.

use std::sync::mpsc;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};
use security_framework::passwords::{
    PasswordOptions, delete_generic_password_options, generic_password,
    set_generic_password_options,
};
use security_framework_sys::base::errSecItemNotFound;
use zeroize::Zeroizing;

use crate::platform::touchid::Biometrics;

#[path = "secure_enclave.rs"]
mod secure_enclave;

const SERVICE: &str = "lockbox.touch-id.v3";
const RETIRED_SERVICE: &str = "lockbox.touch-id.v2";
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
        -8 => "Touch ID is locked after failed attempts — enter the passphrase".to_owned(),
        _ => error.localizedDescription().to_string(),
    }
}

impl Biometrics for MacBiometrics {
    fn domain_state(&self) -> Option<Vec<u8>> {
        if !secure_enclave::available() {
            return None;
        }
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
        purge_legacy(account);
        let envelope = match generic_password(envelope_options(account)) {
            Ok(bytes) => bytes,
            Err(error) if error.code() == errSecItemNotFound => return Ok(None),
            Err(error) => return Err(keychain_error(error)),
        };
        // Keychain returns only ciphertext. The enclave itself must authorize
        // decryption with a fresh context, independent of evaluatePolicy.
        let context = unsafe { LAContext::new() };
        unsafe { context.setLocalizedReason(&NSString::from_str(reason)) };
        secure_enclave::open(&envelope, account, &context)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn store(&self, account: &str, data: &[u8]) -> Result<(), String> {
        purge_legacy(account);
        let envelope = secure_enclave::seal(data, account).map_err(|error| error.to_string())?;
        // Never pass the plaintext record to Keychain, even in unsigned builds.
        set_generic_password_options(&envelope, envelope_options(account)).map_err(keychain_error)
    }

    fn delete(&self, account: &str) {
        purge_legacy(account);
        let _ = delete_if_present(envelope_options(account));
    }
}

impl MacBiometrics {
    pub(super) fn new(touch_id_vaults: &[String]) -> Self {
        // Retire old unlock keys of every Touch ID vault, opened this run or not.
        // Load/store/delete repeat this for the vault they touch.
        for account in touch_id_vaults {
            purge_legacy(account);
        }
        Self
    }
}

fn envelope_options(account: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(SERVICE, account);
    // File-based Keychain works without provisioned entitlements. Biometric
    // protection belongs to the enclave key, not this ciphertext container.
    options.set_access_synchronized(Some(false));
    options
}

/// The obsolete items lockbox itself created for this vault: never the current
/// envelope, and never items of other vaults or other apps.
fn legacy_options(account: &str) -> [PasswordOptions; 2] {
    // Obsolete protected items go when the running build can access them;
    // unsigned builds cannot address this access group (-34018).
    let mut retired = PasswordOptions::new_generic_password(RETIRED_SERVICE, account);
    retired.use_protected_keychain();
    [
        PasswordOptions::new_generic_password(LEGACY_SERVICE, account),
        retired,
    ]
}

/// Best effort: a failed cleanup never blocks the current envelope, so Touch
/// ID keeps working, and the passphrase never depends on the Keychain.
fn purge_legacy(account: &str) {
    if account.is_empty() {
        return;
    }

    for options in legacy_options(account) {
        let _ = delete_generic_password_options(options);
    }
}

fn delete_if_present(options: PasswordOptions) -> Result<(), String> {
    match delete_generic_password_options(options) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == errSecItemNotFound => Ok(()),
        Err(error) => Err(keychain_error(error)),
    }
}

fn keychain_error(error: security_framework::base::Error) -> String {
    error.to_string()
}

#[cfg(test)]
#[allow(deprecated)] // Inspect the dictionaries passed to Security.framework.
mod tests {
    use super::*;
    use core_foundation::{base::TCFType, boolean::CFBoolean, string::CFString};
    use security_framework_sys::item::{
        kSecAttrAccount, kSecAttrService, kSecAttrSynchronizable, kSecUseDataProtectionKeychain,
    };

    #[test]
    fn legacy_cleanup_targets_only_one_vault_in_only_the_old_services() {
        let service = unsafe { CFString::wrap_under_get_rule(kSecAttrService) };
        let account = unsafe { CFString::wrap_under_get_rule(kSecAttrAccount) };
        let value = |options: &PasswordOptions, wanted: &CFString| {
            options
                .query
                .iter()
                .find(|(key, _)| key == wanted)
                .map(|(_, value)| value.clone())
        };
        let options = legacy_options("vault-a");

        for (options, old) in options.iter().zip([LEGACY_SERVICE, RETIRED_SERVICE]) {
            assert_eq!(
                value(options, &service),
                Some(CFString::from(old).into_CFType())
            );
            // Without an account the query would match every item of the service.
            assert_eq!(
                value(options, &account),
                Some(CFString::from("vault-a").into_CFType())
            );
            assert_ne!(old, SERVICE);
        }
    }

    #[test]
    fn encrypted_envelopes_use_local_keychain_without_restricted_entitlements() {
        let options = envelope_options("test-account");
        let protected = unsafe { CFString::wrap_under_get_rule(kSecUseDataProtectionKeychain) };
        assert!(!options.query.iter().any(|(key, _)| key == &protected));
        let sync = unsafe { CFString::wrap_under_get_rule(kSecAttrSynchronizable) };
        assert!(options.query.iter().any(|(key, value)| {
            key == &sync && value == &CFBoolean::false_value().into_CFType()
        }));
    }

    #[test]
    fn malformed_and_unsupported_envelopes_release_no_data() {
        for envelope in [
            b"not an envelope".as_slice(),
            br#"{"version":2,"enclaveKey":"","ephemeralPublicKey":"","ciphertext":""}"#,
            &[0; 65537],
        ] {
            let context = unsafe { LAContext::new() };
            unsafe { context.setInteractionNotAllowed(true) };
            assert!(secure_enclave::open(envelope, "test-account", &context).is_err());
        }
    }

    #[test]
    #[ignore = "OS probe: requires Secure Enclave and enrolled Touch ID; creates one random encrypted Keychain item"]
    fn keychain_refuses_to_release_key_without_biometric_interaction() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let account = format!("touch-id-probe-{:x?}", random);
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete_if_present(envelope_options(&self.0));
            }
        }
        let _cleanup = Cleanup(account.clone());
        let envelope = secure_enclave::seal(&[7; 32], &account).unwrap();
        set_generic_password_options(&envelope, envelope_options(&account)).unwrap();
        let stored = generic_password(envelope_options(&account)).unwrap();
        assert_eq!(stored, envelope);
        assert!(!stored.windows(32).any(|bytes| bytes == [7; 32]));
        let context = unsafe { LAContext::new() };
        unsafe { context.setInteractionNotAllowed(true) };
        // A real enclave operation must fail authentication after reading the
        // ciphertext. Missing entitlements or malformed data do not prove gating.
        let error = secure_enclave::open(&stored, &account, &context).unwrap_err();
        assert_eq!(error.domain().to_string(), "com.apple.LocalAuthentication");
        assert_eq!(error.code(), -1004, "{error}"); // LAErrorNotInteractive
    }
}
