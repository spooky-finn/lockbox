# Touch ID security and signing

The macOS adapter stores optional vault unlock keys in the data protection
Keychain under `lockbox.touch-id.v2`. Each item uses
`AccessibleWhenUnlockedThisDeviceOnly`, disables synchronization, and requires
`BIOMETRY_CURRENT_SET`. Security.framework enforces biometric authentication
when returning the secret. The read uses a fresh `LAContext` for each request;
an earlier application-level fingerprint check does not authorize retrieval.
There is no device-password or unprotected Keychain fallback.

Adding or removing enrolled fingerprints invalidates access. The application
also checks the saved enrollment state and requires the master password after
seven days. That seven-day check is application policy, not a Keychain expiry.
The root signing-key capsule retains its independent password encryption.

Data protection Keychain access requires an appropriately signed app with a
provisioned Keychain access group. An unsigned or ad-hoc development build can
fail with `errSecMissingEntitlement` (-34018). The app reports this failure and
does not save an unprotected replacement. Password unlock remains available.
Release packaging must supply the signing identity, matching Keychain access
entitlements, and provisioning profile; this repository does not supply those
developer-account credentials.

On startup, the app attempts to delete all legacy generic-password items with service
`lockbox`, including items for vaults not opened during that run. It never reads
those secrets. Reads and writes retry cleanup and refuse to proceed if cleanup
fails. The separate `lockbox.profile-witness.v1` service is unaffected.
After upgrading, unlock each vault with its master password to recreate its
protected unlock item if Touch ID was enabled. Migration cannot revoke keys an
attacker copied before the upgrade.

## Validation

`cargo test --locked --lib touchid` checks the workflow failure paths and the
Keychain query configuration without accessing the user's Keychain.

The ignored `keychain_refuses_to_release_key_without_biometric_interaction` test
requires a signed test executable with the same Keychain entitlements and an
enrolled fingerprint. It creates one random test item, attempts an actual
Keychain read with interaction disabled, requires an authentication failure,
and removes the item. Missing entitlements do not count as a successful check.
Run it explicitly in that signed environment:

```sh
cargo test --locked --lib keychain_refuses_to_release_key_without_biometric_interaction -- --ignored
```

Before release, also verify successful biometric unlock, cancellation, disabling
Touch ID, enrollment changes, legacy-item cleanup, and password-based refresh
on a signed app. Confirm that an independent Keychain reader cannot obtain the
protected item without satisfying its biometric access control.

Apple documents [the data protection Keychain and signing requirements](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains)
and [the current-enrollment biometric constraint](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/biometrycurrentset).
