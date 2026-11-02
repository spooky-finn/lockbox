# Touch ID storage

Touch ID works with `cargo run --locked` on a Mac with a Secure Enclave and an
enrolled fingerprint. It does not require an Apple signing identity, Keychain
access entitlements, or a provisioning profile. macOS builds compile a small
CryptoKit bridge using the Swift compiler from Xcode Command Line Tools.

## Protection

Each optional vault unlock record is encrypted using AES-256-GCM. A new ephemeral
P-256 key performs ECDH with a Secure Enclave public key; HKDF-SHA256 derives the
encryption key using the domain `lockbox.touch-id.v3` and the vault identifier.
The identifier also authenticates the ciphertext, preventing envelope substitution
between vaults. The ephemeral private key is discarded after encryption.

The Secure Enclave private key uses `WhenUnlockedThisDeviceOnly` and the
`privateKeyUsage` + `biometryCurrentSet` access-control flags. CryptoKit exports
only its encrypted, device-bound `dataRepresentation`. Decryption must reconstruct
this key and perform ECDH inside the enclave. That operation requires an enrolled
fingerprint; an earlier application-level authentication check cannot authorize it.
Every read uses a fresh `LAContext`. Cancellation and authentication errors release
no vault key. There is no device-password or software-only decryption fallback.

The file-based login Keychain stores the opaque enclave representation, ephemeral
public key, and ciphertext under `lockbox.touch-id.v3`, with synchronization
disabled. It never receives the plaintext vault unlock record. Storing ciphertext
here avoids the provisioned entitlements required by the data protection Keychain.
The enclave's encrypted representation is device-bound, rather than bound to an
app signing identity: another process that obtains the envelope can request
authentication on the same Mac, but still needs a fingerprint to decrypt it.

Adding or removing enrolled fingerprints invalidates access. The application
also checks the saved enrollment state and requires the master password after
seven days. That seven-day check is application policy, not an enclave expiry.
Password unlock refreshes the encrypted envelope without a biometric prompt:
encryption needs only the enclave public key. The root signing-key capsule
retains its independent password encryption.

## Migration

Startup, reads, and writes delete all legacy generic-password items with service
`lockbox`, without reading their secrets. Cleanup failure stops reads and writes.
The separate `lockbox.profile-witness.v1` service is unaffected.

Obsolete `lockbox.touch-id.v2` data protection Keychain items are also deleted
when the running build has access. Unsigned builds ignore the missing-entitlement
error for deleting those already protected items; they never read them.
After upgrading, unlock each vault with its master password to create a new
encrypted envelope if Touch ID was enabled. Migration cannot revoke keys an
attacker copied from older unprotected storage.

## Validation

`cargo test --locked --lib touchid` checks workflow failure paths,
Keychain query configuration, and malformed or unsupported envelopes without
accessing the user's Keychain or requiring hardware authentication.

The ignored `keychain_refuses_to_release_key_without_biometric_interaction` test
requires a Secure Enclave and enrolled Touch ID. It creates one random encrypted
Keychain item, confirms the stored bytes are the envelope, and attempts real
enclave decryption with interaction disabled. Only the LocalAuthentication
`notInteractive` error counts as proof of biometric enforcement. Missing signing
entitlements, missing items, and malformed envelopes fail the probe. The item is
removed afterward. No signing setup is required:

```sh
cargo test --locked --lib keychain_refuses_to_release_key_without_biometric_interaction -- --ignored
```

Before release, also verify successful biometric unlock, cancellation, disabling
Touch ID, enrollment changes, legacy-item cleanup, and password-based refresh
on a real Mac.

Apple documents [Secure Enclave key protection](https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave),
[CryptoKit's encrypted enclave representation](https://developer.apple.com/documentation/cryptokit/storing-cryptokit-keys-in-the-keychain),
and [Keychain signing requirements](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains).
