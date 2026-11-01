# Profile protection and fresh-device recovery

New shared profiles use encrypted bootstrap schema **5** inside the versioned
`.lbx` container. Signed remote objects use one unversioned protocol; previous versioned
histories are unsupported. Local file generations
and witnesses are never uploaded. Local vaults now write v4; v1/v2 remain readable. Shared profiles also read v3.
The file version prevents older clients from discarding entry attachments.
Schema 4 preserves the [durable invitation ledger](sync-invitations.md); schema 5
preserves [controller transition approvals](sync-control-recovery.md).
Protected schema-3 profiles retain their witness and upgrade when using sharing.

## Independent installation evidence

The authenticated `FileGeneration` contains a random binding, increasing `u64`
generation and previous complete encrypted-file digest. Independent `Witness`
records contain the binding, profile ID, highest generation and its digest.
Neither type contains private keys, provider credentials or record plaintext.

The profile ID hashes the stable file salt with `lockbox.profile-id.v1\0`.
The complete header and ciphertext are hashed with `lockbox.profile-file.v1\0`.
These local durability identifiers grant no space authority.

Witnesses are addressed by the exact device fingerprint. An exclusive signer
lease lasts for the unlocked session, including when the profile is read-only.
Copies at different paths contend for one lease. Moving the latest file after
closing it remains supported.
Removing or replacing a held lock pathname invalidates the old lease; it cannot
remain usable while a second process acquires a newly created lock file.

| Platform/host | Storage |
| --- | --- |
| macOS | Login Keychain service `lockbox.profile-witness.v1`, device fingerprint account |
| Other desktop platforms | Private installation directory under `dirs::data_local_dir()/lockbox/signer-witnesses` |
| Unit tests and offline example | Explicit memory adapter, retained across reopening |
| Live S3 probe | Explicit temporary directory witness, without personal Keychain items |

Persistent backends hold a kernel file lease per device in the installation
directory. Keychain denial never falls back to another backend. Keep directory
witnesses separately from profile backups. A custom `WitnessStore` must provide
exclusive leases across its processes, durable compare-and-replace, distinct
missing/error outcomes and independence from profile restores. `MemoryWitness`
is nonpersistent and unsuitable for an actual installation.

## Commit and reconciliation

A new locally generated identity reserves generation zero before its first file
commit. Opening an existing profile never creates a missing witness.

1. Verify the current file digest and reconcile its existing witness.
2. Write generation `g + 1`, naming the exact previous digest, to a private
   temporary file and synchronize its contents.
3. Preserve the previous encrypted file through atomic private backup replacement.
   Temporary and backup paths never truncate through symlinks.
4. Rename the new profile and synchronize its directory.
5. Update the independent witness, then allow signing/publication.

Events use this boundary for both the unsigned intent and the signed event/view/
outbox. Merge, membership, resolution, rebase, incoming history and acknowledgement
commits share the same protected file adapter.

| Observed state | Behavior |
| --- | --- |
| Generation/digest equal the witness | Resume normal operation |
| Exactly one generation ahead, naming the witnessed digest | Synchronize and advance the witness before resuming |
| Witness update persisted but response was lost | Recognize equality; do not reset or repeat the generation |
| Older file, generation gap, changed bytes at equal generation, different binding/profile | Preserve readable data; block signing and exchange |
| Missing witness | Preserve readable data; require a new admitted device |
| Denial, unavailable or corrupt store | Preserve readable data; retry without reenrollment |

The adapter remembers a completed rename even when directory sync or witness
acknowledgement fails. Retries cannot reuse that generation. Failed incoming
commits also retain their candidate separately from the previous in-memory view.
Restart reads the actual file, retaining a committed revocation or integrity fault
even if its witness update was interrupted.

`Vault::signing_error` exposes the cached reason for read-only operation.
`reconcile_profile` retries store access and the one-generation catch-up without
reenrolling an old signer. The desktop cancels pending exchange, shows localized
recovery notices and disables mutation actions while blocked. Retry can reconcile
the witness without installing incoming history into an active draft.

## Fresh-device recovery

Unlock the backup for reading. Obtain the current descriptor and control hash
through an authenticated owner/peer channel independently of that backup and the
provider. Writable peers export them using `space_descriptor()` and `space_anchor()`.

Call `ProfileStore::restore_device` with `DeviceRecovery`:

```rust,ignore
let recovered = profiles.restore_device(
    new_path,
    &new_master_password,
    kdf,
    DeviceRecovery {
        source: &readable_backup,
        recovery_password: &separate_recovery_password,
        descriptor: authenticated_descriptor,
        control_anchor: authenticated_control_hash,
        configuration: SharedProfile {
            username,
            space_name,
            provider: independently_provisioned_credentials,
        },
    },
)?;
```

The separate recovery password opens the root capsule. Fresh Ed25519 and HPKE
device keys are certified under the same user root. The new profile contains a
new installation binding, empty join journal and pinned minimum control hash.
It does not import old signing keys, counters, outbox, epoch keys or a readable
backup snapshot as authoritative current content.

The owner verifies the new certificate independently and explicitly replaces the
old device in a roster transition. Before admission the new device cannot publish.
After admission it receives the current checkpoint and epoch grant, without old
epoch keys. Earlier plaintext history is owner-attested. Backup records and
excluded drafts remain in the readable source for explicit review; they never
silently overwrite the current checkpoint.

Other hosts can use `Machine::join_at`: replay stays non-ready until the verified
control chain contains the exact required head. Discovery fetches it directly
even if LIST omits it. An older valid prefix cannot authorize a write while that
anchor is missing.

Bootstrap-only schema-1 profiles activate using the recovery password, a fresh
device and an atomic protected import intent. Existing schema-2 shared profiles
open read-only and require fresh-device admission. Older applications reject
schemas 3/4 instead of dropping their protection metadata; a schema-3 application
also rejects schema 4 so invitation cancellations and retry artifacts survive.

## Limits and evidence

The witness detects ordinary backup rollback when independent installation
evidence survives. It is not a hardware monotonic counter. Restoring both profile
and witness, or compromising the OS, requires independent peer history or
fresh-device recovery. Provider omission cannot supply that evidence.

This identity recovery does not transfer ownership/control, automatically replace
a lost controller, recover a lost root capsule or replace a current content
backup. Invitation/recovery screens and explicit controller recovery remain
separate work in [the production plan](sync-production-plan.md).

Tests cover rollback, missing bindings, authenticated re-encryption, generation
gaps, enrollment and intent/witness crash states, lost acknowledgements,
revocation restart, store denial/corruption, cross-process leases, sidecar
symlinks, UI read-only behavior and fresh-device admission.

The normal suite substitutes the credential store. An explicit macOS probe
creates and removes one random witness item containing only fictional public
metadata:

```sh
cargo test --locked --lib infra::witness::keychain::tests::keychain_witness_round_trip -- --ignored --exact
```

That OS round trip and cleanup passed on 2026-11-01. It verifies the actual
Keychain adapter; it does not replace a live Storj exchange, physical power-loss
testing or independent security review.
