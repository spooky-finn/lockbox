# Addressed invitations and admission

Shared spaces use one experimental, unversioned protocol. Causal events, HPKE,
checkpoints and durable signing require signed consent for every added device
certificate. This implementation remains separate from the broader
[protocol proposal](lockbox-protocol.md). The desktop implements the invitation
and member-management flows below; other production release gates remain open.

## Desktop workflow

The owner opens **Sharing and participants** from an unlocked cloud profile's
sidebar. A new participant selects **Join a space** on the welcome screen,
enters their name and independent credentials for the owner's Storj storage,
then chooses master and recovery passwords. The app creates an encrypted local
staging profile with fresh identity/device keys and opens the joining wizard.
The participant does not need to name or manually create a separate space.
Provider credentials remain required and are never included in sharing files.
Existing records or signing history cannot be discarded to join another space.

1. Recipient selects **Create and save join request** and sends the saved device
   file to the owner. Compare user/device fingerprints through a trusted channel.
2. Owner selects **Open sharing file**, reviews the detected device request,
   confirms fingerprints, chooses the deadline and selects **Save read-only
   invitation** or **Save editor invitation**. An additional device of the
   owner's own identity uses **Save invitation for my device**.
3. Owner sends the saved invitation file and shares its 64-character verification
   code through a separate trusted channel. The code remains visible after
   saving and binds the full space descriptor and signed control anchor.
   The recipient screen does not display the expected code for copying into its
   own verification field.
4. Recipient selects **Open sharing file**, enters the independently received
   code and selects **Sign and save consent**. Send the saved response to the owner.
5. Owner opens that response using **Open sharing file**, reviews the verified
   device/role and selects **Approve and save confirmation**. Send the saved
   confirmation to the recipient.
6. Recipient opens the confirmation using **Open sharing file** and selects
   **Complete connection**. The app installs the verified checkpoint and begins
   normal synchronization.

A single opener detects all four artifact types by content and retains strict
parsing and signature checks. Artifact creation opens the save dialog directly;
it does not send files to another person. Cancelling that dialog leaves the
artifact available under **Save sharing file**, and failed writes can be retried.
Successful saves advance the wizard to the next handoff, show waiting instructions
and offer **Save another copy**. Pending invitations/consents remain resumable
from the encrypted ledger after restart. A session lock preserves the joining
path, and reopening a profile with pending consent returns to sharing.

The page lists devices and pending invitations after restart. The controller can
change a participant's role across all its devices, cancel a pending invitation,
re-export saved artifacts, refresh an admission confirmation, and remove a
specific device after explicit confirmation. The controlling device cannot be
removed. Removing a device rotates the key; data already received remains on that
device. A removed device needs fresh keys and a new invitation to rejoin.

Save or cancel an active entry/folder draft before changing access. File reads,
parsing and exports run in cancellable workers tied to the originating unlocked
profile. Locking or switching spaces discards stale replies. Exports require a
new filename and never overwrite an existing profile, file or symlink. Access
changes/cancellation clear prior output, including a still-open export dialog.
Record passwords and provider credentials do not appear on the sharing page.

An unadmitted recovered device shows **Waiting for owner admission**, distinct
from removal. Recovery of a copied/read-only profile still uses the fresh-device
API; a complete desktop identity-recovery wizard is pending.

## Exchange

The owner independently checks the recipient's complete device certificate and
user fingerprint. The recipient independently authenticates the complete
`SpaceDescriptor` (space, namespace, owner root and genesis). An artifact or
provider connection alone cannot establish these trust anchors.

1. Owner calls `Vault::issue_invitation(certificate, role, expires_at)`. The
   addressed offer binds a random invitation ID, descriptor, verified control
   anchor, issuing controller, exact recipient certificate, role and optional
   expiry. A public control-chain proof accompanies it.
2. Recipient calls `accept_invitation(invitation, &authenticated_descriptor)`.
   It verifies the proof, controller signature and exact match to its own
   signing/HPKE keys. It signs the offer hash, device ID and a random challenge.
3. Owner calls `admit_invitation(response, now)`. It requires the exact locally
   issued offer, checks cancellation/expiry, verifies consent and reserves a new
   membership control. That control contains both signatures and rotates the
   epoch key with the ordinary encrypted checkpoint and recipient grants.
4. Recipient calls `join_invitation(receipt)`. The controller-signed receipt
   binds the descriptor, invitation, exact acceptance hash and current control
   head. The recipient verifies the proof and pins this head before installing
   the new state. Recovery profiles also require their independent minimum head
   to appear in the proof.

Each artifact has canonical `bytes()` and strict `decode()` methods. Artifacts
contain public identity/control metadata and encrypted grants/checkpoints. They
contain no provider credentials, private device keys or unencrypted record data.
Access to public metadata still reveals member identities and the control chain.

```no_run
use lockbox::{core::sync::Role, infra::vault::{Result, Vault}};

fn connect(owner: &mut Vault, recipient: &mut Vault) -> Result<()> {
    // In a real host, authenticate both values independently before these calls.
    let descriptor = owner.space_descriptor().unwrap();
    let certificate = recipient.device_certificate().unwrap();
    let invitation = owner.issue_invitation(certificate, Role::Editor, None)?;
    let response = recipient.accept_invitation(invitation, &descriptor)?;
    let receipt = owner.admit_invitation(response, 0)?;
    recipient.join_invitation(receipt)
}
```

After admission, normal `synchronize` exchanges immutable objects. The current
checkpoint permits a fresh device to join without old content keys. Writer
profiles may relay owner-signed control metadata received out of band; readers
need no provider PUT permission. Publishing still follows dependency discovery
and the durable outbox boundary.

## Deterministic authority

Replay verifies every embedded offer and acceptance against the authenticated
control prefix. The exact certificate and role must match the added roster entry.
Invitations cannot be consumed twice. A device ID that appeared in any previous
roster cannot be admitted again; device replacement requires fresh signing and
HPKE keys. Additional devices of one user must have the same role. Only devices
of the owner's user identity can have `Owner`; the verified current controller remains the
only membership authority.

`set_space_members` remains the API for changes to existing roles, removal and
key rotation. Adding a device without consent fails with `ConsentRequired`. Pure hosts can use `Machine::prepare_admission` or batch multiple
proofs with `prepare_admissions`, persist the returned intent, then complete and
persist it before publication. `Machine` does not implement local expiry or
cancellation policy: that policy belongs to the durable controller host.

Wire artifacts carry no protocol version or profile selector. Control/change
signatures use fixed, distinct domains. Previous versioned artifacts are rejected;
there is no upgrade, downgrade or compatibility path. `change.json` pins the
canonical wire bytes and an independently generated signature.

## Persistence and local policy

Bootstrap schema **4** stores issued invitations, exact responses, cancellation,
reserved admissions and receipts inside the encrypted profile. Schema-3 profiles
upgrade on first sharing operation and retain the same independent witness.
Older applications reject schema 4 rather than silently dropping its ledger.

Every returned offer, response or receipt is committed before export. Repeating
the same operation after an interrupted file save or restart reuses the stored
artifact. A different response to an already reserved invitation is rejected.
Membership intent recovery completes the exact reserved control before the
receipt can be exported. Repeating a join preserves later local edits.

Ordinary retries return the originally persisted receipt. If the waiting
recipient has independently learned a newer minimum head, the owner can call
`refresh_invitation_receipt(id)` to persist and export a new confirmation of its
current verified head. This performs no additional admission or epoch rotation.

`cancel_invitation` durably cancels an unconsumed offer. Expiry is an exclusive
owner-side deadline: `now >= expires_at` rejects a new reservation. An already
reserved admission completes even after expiry; removal then uses a new roster
transition. Replicas never use wall clocks to grant or reject event authority.

The local ledger permits at most 128 total records and 64 MiB of canonical data.
`forget_invitation` releases cancelled, expired or admitted issuance records;
pending/reserved ones cannot be forgotten. Forgetting disables local artifact
retries for that ID; shared admission proofs remain in signed history. Public
invitation and receipt decoding is bounded to 96 MiB encoded, and the underlying
control proof remains subject to the 4,096-object/64 MiB journal limits.

A signed receipt detects unsigned proof truncation. It cannot prove that an
unreachable controller has not signed a later revocation. A previously valid
signed prefix can be withheld by the provider or artifact sender. Freshness
requires independently authenticated current anchors and subsequent discovery;
the [profile witness](profile-protection.md) protects previously observed local
state, not knowledge of events that have never arrived.

## Evidence and remaining work

Tests exercise independent recipients, wrong roots, substituted signatures/roles,
missing consent on received controls, rejection of versioned artifacts, retired device
IDs, signed-head truncation, expiry/cancellation, exact retries after restart,
file failures, independently pinned newer heads and explicit receipt refresh,
and a witness acknowledgement failure after the receipt file
commit. The shared memory/live acceptance scenario now uses the full invitation
exchange before concurrent edits, resolution, reader restrictions and revocation.

Controller transfer/recovery have a [separate API](sync-control-recovery.md).
The desktop identity/control recovery wizards, scalable
proof compaction, a live Storj run and independent security review remain
[production release work](sync-production-plan.md).
