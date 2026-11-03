# Addressed invitations and admission

Shared spaces use one experimental, unversioned protocol. Causal events, HPKE,
checkpoints and durable signing require signed consent for every added device
certificate. This implementation remains separate from the broader
[protocol proposal](lockbox-protocol.md). The desktop implements the invitation
and member-management flows below; other production release gates remain open.

## Desktop workflow

**Invite with a link** is the primary desktop flow. The owner opens **Sharing
and participants** from an unlocked cloud profile. A new participant selects
**Join a space** on the welcome screen, enters their username and independently
provisioned credentials for the owner's Storj endpoint/bucket, chooses a local
master password and saves their mnemonic. This creates fresh device keys in an
encrypted staging profile. Normal space sync waits until joining completes.
The local profile is never copied or uploaded to admit another device.

1. Owner selects **Create invitation link**, waits for **Link ready to send**,
   then selects **Copy invitation link**. Send the short
   `lockbox://i/<encoded space name>/<ticket hash>` text to the participant.
   **Retry invitation upload** retries an interrupted upload; copying stays
   disabled until both the signed ticket and control proof are published.
   Use **Copy verification code** to deliver the complete 64-character code
   through a second trusted channel, separately from the link. A link defaults to seven days; one and
   thirty days are available under **Invitation validity**.
2. Participant pastes the link into **Join with a link**, enters the separately
   received code and selects **Verify and send request**. The app downloads the
   signed ticket from the configured storage, checks its complete hash, signature,
   space name and independent code, then verifies the pinned control proof before
   sending a device-signed, encrypted request containing username and complete
   device certificate.
   Links are pasted into Lockbox; OS/browser URL-handler registration is not
   required or claimed by this implementation.
   Pending downloads survive restart. **Edit link or code** returns to the form
   with the entered values so a typo can be corrected before any request is sent.
   Previously issued `lockbox://invite/...` links remain accepted.
3. Participant selects **Copy message for administrator** and sends their
   username, complete User ID and complete Device ID through the second trusted
   channel. The administrator sees the signed username and both IDs in the
   pending request and compares the complete values. Usernames are self-declared
   labels; signatures and the independent ID check establish the device binding.
4. Administrator checks the verification box, chooses the role and selects
   **Approve this device**. New users default to Read only; an existing user's
   additional devices retain that user's role. Approval captures the exact
   signed request hash and selected role. Other requests cannot inherit it.
   **Decline request** rejects only the chosen request.
5. Participant reviews the offered role and selects **Confirm and connect**.
   The app signs the exact addressed offer and automatically delivers consent,
   the controller's admission receipt and a signed completion acknowledgement.
   The administrator approves once. Ordinary admission still rotates the epoch
   and requires the original consent/receipt checks. No files are exchanged in
   this path.

Both applications must keep the relevant profile unlocked during their part of
this exchange. Pending requests, exact approvals, consent and encrypted outgoing
messages survive restart. An opening profile with a pending link returns to the
joining page. **Retry connection exchange** republishes the stored bytes;
retries do not consume another invitation or create another membership change.
Locking drops session workers; replies are bound to the originating profile and
device. Clipboard copies use the existing concealed-history/20-second guard and
exclude credentials, private keys, mnemonic and record contents.

The controller pins the link to one request when approving it. A signed completion
acknowledgement retires the link; cancelled and expired links also stop new
admissions. An already reserved membership transition follows existing recovery
semantics and cannot be undone by cancelling the link: remove the device through
normal member management. Completed links can be removed from the local list
after their terminal status has been published. Removing history cannot make an
unknown/consumed ticket authorise another admission.

## Storage and trust boundaries for links

Short links contain the base64url-encoded space name and complete 256-bit ticket
hash (43 base64url characters), not the ticket itself. A space named `Team`
produces a 62-character link. No hash truncation or URL-shortening service is used.
The exact signed ticket is published through the existing bounded/chunked mailbox
adapter at `spaces/<escaped space name>/pairing/<ticket hash>/ticket.json`.
The app uses only the recipient's configured endpoint and bucket to resolve it;
the downloaded ticket must match the locator hash and space name before it can
advance joining. Missing or overwritten data cannot substitute another ticket;
it can prevent joining. A provider acknowledgement establishes upload completion,
not a promise that the provider will retain or serve those bytes.

Link tickets contain public routing, an owner-certified controller certificate,
a random session ID, control anchor/proof hash, expiry and controller signature.
They contain no provider credential, decryption secret, private device key or
epoch key. Link possession grants no S3 operation and no membership. The complete
verification code binds the exact ticket, including owner identity, anchor,
routing and certified encryption key. It must be authenticated independently.
The subsequent addressed invitation must extend this pinned control anchor.

Each client uses its own configured Storj credentials. The ticket's endpoint and
bucket must match that explicit configuration before any contact message is sent.
Ticket resolution reads only the configured endpoint and bucket. No arbitrary
URL from a link is fetched, no credentials are forwarded to a new endpoint, and
redirects remain disabled. The adapter uses a separate
`spaces/<escaped space name>/pairing/<random session>/` directory. The controller
needs GET/PUT and LIST for request discovery there; joining clients need GET/PUT
for their mailbox messages. A Reader can have write permission only for this
pairing prefix, retaining GET/LIST without PUT for ordinary space objects.
Provision these permissions independently; copying the owner's unrestricted
credentials is not part of joining. No presigned bearer URLs are issued.

Requests, offers, consent, receipts and completion acknowledgements use
HPKE-encrypted envelopes with separate domains and ticket/request/type/recipient
context. Requests additionally bind the username and exact certificate with the
device signature. The signed ticket, bootstrap control proof and signed terminal
status remain public metadata, like ordinary signed control objects. Storage sees
object names, sizes and traffic; no claim of hidden membership metadata or
recipient-compromise forward secrecy is made.

Large documents use 1 MiB hash-checked chunks and bounded manifests, separately
from the normal 4 MiB immutable sync object contract. The normal proof/history
limits still apply. Short links are bounded to 512 bytes, signed tickets to 8 KiB,
legacy link parsing to 16 KiB, contacts/manifests to
64 KiB, and link state/outgoing caches to 64 MiB with 16 sessions and 32 requests
per session. Request polling is bounded and partial uploads can be retried.
Providers can withhold, overwrite or replay bytes; signatures, exact approvals,
minimum heads and the local witness protect authority, not availability. A link
becoming inactive does not erase already stored ciphertext or revoke separately
provisioned S3 credentials.

## File fallback

**Connect using files** expands the existing three-step recipient/controller
wizards. Their opener still detects all four artifact types by content:
recipient certificate, addressed invitation, signed consent and admission
confirmation. The recipient sends a device file; the owner verifies fingerprints
and sends an invitation; the recipient authenticates the owner code and sends
consent; the owner returns the signed confirmation. The recipient imports it to
finish. Save retry, explicit sent-file handoffs, independent verification and
exact durable artifacts remain supported. This fallback also permits offline
handoffs; link delivery requires the configured provider.

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
need no provider PUT permission for ordinary space objects. Link onboarding uses
separately provisioned pairing-directory write permissions. Publishing still follows dependency discovery
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

Bootstrap schema **7** added the durable link ledger and exact encrypted delivery
cache. Schema **8** additionally retains short-link publication and pending ticket
resolution. Existing link sessions gain the same signed ticket's short delivery
on their next exchange, without changing their verification code. Profiles
upgrade on a link operation; older applications reject the new schema. The
existing witness also protects link approvals, consumption and cancellation
against local profile rollback. Membership wire artifacts remain unchanged.

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
