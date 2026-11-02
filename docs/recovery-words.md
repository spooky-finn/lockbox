# Recovery words

The desktop uses **one private identity key across all spaces**. First-time setup
starts with **12 English BIP39 words**, before selecting local or cloud sync.
Generate words or enter an existing valid phrase, confirm that the words are
saved, and choose an identity password. The identity is saved independently of
any space, even if connection setup is cancelled. Later Create and Join flows
unlock and reuse this identity; they do not offer key regeneration. Switching
between Create and Join also preserves the selected words during first setup.
The app generates 128 bits of operating-system randomness and the BIP39 checksum.
It never derives these words from the master password. Save the words in order before creating
the vault; they are hidden until requested and are not retained in space files.
Going back to sync settings preserves the confirmed phrase and clears typed
passwords. The native local-file picker also preserves the chosen recovery key.
The master password remains the everyday unlock credential. No additional
BIP39 passphrase is used, so the 12 words alone reproduce the recovery keys.

The local `identity.key` beside the preferences file holds the root key and, when
available, its words. It uses Argon2id, a separate HKDF encryption domain, and
XChaCha20-Poly1305 with authenticated headers and independent password/recovery
key slots. Files are private to the OS user. Concurrent setup cannot overwrite
an existing identity. Forgetting a vault path does not remove the identity.
The identity password is separate from individual space-file passwords. Its
**Forgot master password?** action uses the same original words to reset only the
identity password, preserving the root and leaving space passwords unchanged.

An existing shared profile can supply its original root when first starting
Create/Join. Adoption preserves its keys and the saved installation identity.
Profiles created earlier with different identities
retain their existing keys. Their independent signed histories cannot be
silently reassigned. Adoption cannot reconstruct words from a root key: if the
original words exist, recovering the identity with them saves them for future
space recovery slots. An older random root has no corresponding recovery words.

On the lock screen, select **Forgot master password?**, enter the words, and
choose and confirm a new master password. The original password is not needed.
Recovery preserves the user identity, device, records, and Touch ID data key.
For cloud profiles, the new password also protects the root capsule. Previous
backup files retain the password they had when saved.

Local password recovery requires the vault file or a backup. Enrolled owner cloud
profiles also support [cloud-only recovery](cloud-recovery.txt): original words,
endpoint, bucket and independently retained credentials recover published data
without that file. This creates a fresh controller rather than copying old device
keys. Provider-only recovery requires explicit trust in the available snapshot;
an independent current anchor is needed for stronger freshness guarantees. Words
cannot recover deleted cloud objects or unsynced edits. Words are
shown during creation, so keep a separate copy before continuing. Existing
profiles without recovery key slots retain their existing unlock credentials;
words cannot be retroactively generated for their random root keys.

## Identity derivation

[BIP39](https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki) converts the
mnemonic to a 64-byte seed using PBKDF2-HMAC-SHA512, 2048 iterations, and an empty
passphrase. The Ed25519 root is the left 32 bytes of
`HMAC-SHA512(key="ed25519 seed", data=seed)`, the
[SLIP-0010](https://github.com/satoshilabs/slips/blob/master/slip-0010.md) master
key at path `m`. Lockbox keeps its existing Ed25519 signatures and user ID
fingerprint domains. Device signing/HPKE keys and space keys remain independently
random. Restoring an identity on a new device never derives or copies an old
device signing key from the words.

`access::mnemonic::RecoveryPhrase` validates exactly 12 English words and their
checksum. The pinned `bip39` dependency enables zeroization. Mnemonics, entropy,
seeds, and derived keys are wiped on drop in Rust. Recovery errors do not include
the input words. The UI uses no automatic clipboard export.

## File format v8

Recoverable local vaults and cloud profiles use version 8; existing v1–v7 files
remain readable.
Older clients reject v8 instead of discarding recovery slots. The original
90-byte header is followed by two 72-byte key slots, then the encrypted payload:

| Offset | Bytes | Value |
| --- | --- | --- |
| 0 | 90 | Original header, version 8, KDF, salt, password check, payload nonce |
| 90 | 72 | Password slot: 24-byte nonce and 48-byte encrypted data key |
| 162 | 72 | Recovery slot: 24-byte nonce and 48-byte encrypted data key |
| 234 | remaining | Authenticated encrypted JSON payload |

A random 32-byte data key encrypts the payload. The existing Argon2id/HKDF
password derivation wraps this key in the password slot. A separate recovery
wrapping key is derived with HKDF-SHA256 from the BIP39 seed, the file salt, and
`lockbox.recovery-wrap.v1\0`. Each slot uses XChaCha20-Poly1305 and its own random
nonce. Slot associated data is `lockbox.key-slot.v1\0`, the file salt, and the
slot kind (`password` or `phrase`). The whole 234-byte header authenticates the
payload, binding both slots and the password check to the encrypted records.

Password reset rewraps the same data key with a fresh slot nonce and uses the
existing atomic save. Cloud profiles also reseal the root capsule and retain
their witnessed save. The file salt, installation binding, and device identity
stay stable. Recovery never resets or bypasses independent rollback witnesses.
For cloud profiles on another installation or with an untrusted backup, the app
recovers read-only access and reports that password
reset requires admission of a fresh device. Device/controller recovery still
requires independently authenticated control anchors and the established
admission process.

Hosts can use `Vault::create_with_recovery_phrase` for local vaults,
`ProfileStore::create_shared_with_phrase` for cloud profiles,
`ProfileStore::open_with_recovery_phrase`, and
`Vault::reset_password_with_phrase`. The original creation/recovery APIs remain
compatible; only the phrase-aware creation API installs v8 recovery slots.
