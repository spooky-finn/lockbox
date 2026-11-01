# Controller transfer and recovery

Shared spaces use one experimental, unversioned protocol with explicit controller
transitions and separate approval domains. It remains distinct from [the proposal](lockbox-protocol.md); independent
review and a stable interoperability release remain open gates.

The owner identity stays fixed. Control handoff moves membership authority
between devices certified by that same owner root. It does not transfer root
ownership to another user.

## Normal handoff

First admit the incoming controller as another owner device through the normal
[addressed invitation](sync-invitations.md) flow. Its private keys remain local.

1. Old controller calls `begin_control_transfer(target_device)`. Vault persists
   the exact next epoch, roster, grants, checkpoint and close frontier before
   exporting a `ControlTransfer` offer. Ordinary mutations stop while this
   successor slot is reserved; the roster remains unchanged.
2. Incoming controller calls `accept_control_transfer(&offer)`. It verifies the
   old controller's offer signature, exact descriptor/current predecessor,
   admitted owner certificate and checkpoint against replayed closed history.
   Its approval is committed before export.
3. Old controller calls `finish_control_transfer(approval)`. It persists the
   approval before emitting and committing the final control signature/outbox.
   Repeating an exact completed request is idempotent.
4. Synchronization distributes the event. Only the new controller can issue the
   next membership control. The old device retains ordinary data access until
   explicitly removed.

`ControlTransfer::bytes/decode` and `Signed<ControlApproval>::bytes/decode` provide
canonical artifacts. The offer contains public metadata and encrypted dependency
history, never private device keys or credentials. Encoded offers are bounded to
96 MiB and the underlying 4,096-object/64 MiB cache budget. The incoming approval
cache permits 128 records per profile.

`cancel_control_transfer` releases a reservation only before the incoming approval
is attached, when no final control signature can exist. Later edits outside the
fixed close frontier remain as excluded drafts for explicit review/rebase.

## Lost-controller recovery

Three distinct capabilities are required: owner authority (a root capsule and
its password: the master password for new desktop profiles, or the original
recovery password for older profiles), current readable content, and an independently
authenticated `RecoveryAnchor`. The anchor contains the complete descriptor,
current control hash and exact data frontier. A stale backup with the same
control head but older data fails the frontier comparison.

Obtain the complete anchor from `Vault::recovery_anchor()` on a trusted writable
peer and authenticate it separately from the provider/backup. It commits to
encrypted event hashes, exposing no plaintext password-derived state digest.
Missing content, dependencies, pending receive commits and frozen history pause
recovery instead of producing an empty replacement checkpoint.

```no_run
use lockbox::{core::sync::RecoveryAnchor, infra::vault::{
    ControllerRecovery, KdfParams, ProfileStore, Result, SharedProfile, Vault,
}};
use secrecy::SecretString;

fn recover(
    owner_backup: &Vault,
    current_content: &Vault,
    independent_anchor: RecoveryAnchor,
    root_password: &SecretString,
    new_master: &SecretString,
    configuration: SharedProfile,
) -> Result<Vault> {
    ProfileStore::default().recover_controller(
        "fresh-controller.lbx", new_master, KdfParams::default(),
        ControllerRecovery {
            authority: owner_backup, content: current_content,
            recovery_password: root_password, anchor: independent_anchor,
            configuration,
        },
    )
}
```

An imported/read-only owner backup can supply root authority; a current reader
can supply content. Both sources remain unchanged. Recovery generates fresh
signing/HPKE keys, removes the lost controller, preserves other members/roles,
rotates the epoch key and encrypts the complete verified projection, including
conflicts and tombstones. Private device keys and writable sequence state are
never copied. The new outbox is rebuilt from actual signed dependencies.

The owner root approves the prepared statement; the fresh device signs the
complete control including approval. Existing peers still compare the checkpoint
with their replayed history wherever keys permit. A new device's historical
plaintext verification remains limited, as with admission. Closed ciphertext can
be relayed after public signature, role, scope, sequence and ancestry checks;
relay provenance does not claim verification of unavailable old plaintext.

## Persistence and restart

Bootstrap schema **5** preserves controller approval state. Earlier protected
profiles upgrade on control operations; sharing never downgrades them. Older
applications reject the schema instead of dropping reserved state.

Root recovery commits the fresh encrypted profile/snapshot/intent first, reserves
the statement in a separate owner-root signer journal, persists root approval,
then commits the fresh-device signature, event, projection and outbox before
publication. The root journal uses an exclusive, domain-separated address for
the complete space descriptor. It binds the target profile, statement and epoch.
Exact retry resumes. Another file cannot reserve a competing claim. A later
claim requires both a newer epoch and a verified chain containing the previous
recovery statement; numeric ordering alone grants no authority.

The root journal reuses the durable Keychain/directory CAS adapter under its own
record namespace and never enters file-generation reconciliation. If creation
fails after writing a target, reopen that same target. A persisted root approval
finishes automatically. An intent awaiting approval remains readable; resume
with `approve_controller_recovery(root_password)`. Do not retry a reserved root
claim by creating another target.

The independent journal protects local accidental duplication while intact. It
is not a globally shared root counter. Full-machine rollback or simultaneous
recovery on independent installations requires peer comparison. Competing valid
successors freeze as a control fork; no timestamp, hash or recovery preference
silently overrides a known branch.

Kernel leases explicitly unlock on owner drop. Duplicated or briefly inherited
descriptors must not extend a finished session, consistent with the shared-lock
references in [Apple's flock contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html).

## Wire boundary

`Control` adds optional `transition` and `approval`, omitted on ordinary controls.
An approval binds the complete descriptor, author identity and statement hash. That
hash covers canonical control fields with only `approval` omitted, under
`lockbox.sync.control-statement\0`. The final author signs the complete control
including approval bytes. This nested encoding avoids circular hashes and is
explicitly separate from the proposal's signature-envelope encoding.

| Signature | Domain |
| --- | --- |
| Ordinary/transfer controller | `lockbox.sync.control\0` |
| Transaction | `lockbox.sync.change\0` |
| Transfer offer | `lockbox.sync.control-transfer-offer\0` |
| Incoming controller approval | `lockbox.sync.control-transfer-approval\0` |
| Recovery root approval | `lockbox.sync.control-recovery-root\0` |
| Recovered-device final author | `lockbox.sync.control-recovery-device\0` |

Root authority comes from the verified predecessor's owner certificate, never
from an untrusted recovery claim. Transfer requires the exact already admitted
owner device's approval. Recovery permits up to 512 closed heads, bounded by the
roster limit, so a surviving reader need not forge a writer merge. Other
transitions retain the 64-head folding path and all object/cache budgets apply.

## Evidence and remaining work

Tests cover missing/substituted approval, stale content/frontier, restart,
old-controller authority loss, retained late drafts, root/normal forks in both
orders, lost witness acknowledgements, duplicate root claims and unpublished
dependency relay. Public-API integration uses an old read-only owner backup plus
current reader-held content, then resumes edits and synchronization. The unversioned wire format has an independently generated signature fixture.

Root ownership transfer, automatic fork repair, desktop handoff/recovery wizards,
large-history compaction, global freshness proofs, live Storj validation and
independent review remain unfinished. An already frozen control fork is not
resolved by choosing a recovery branch. The [production plan](sync-production-plan.md)
remains active.
