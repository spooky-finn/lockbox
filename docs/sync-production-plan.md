# Production synchronization delivery plan

This is the release work for Lockbox's multiuser, local-first event engine over
Storj/S3. Passing an offline demonstration does not satisfy this plan. The
unversioned sync protocol remains experimental until the release gates below
have evidence. The [protocol proposal](lockbox-protocol.md) defines the broader
design; [sync-engine.md](sync-engine.md) describes implemented behavior.

## Required invariants

1. The same authenticated event set and pinned trust anchors produce identical
   membership, accepted changes, conflicts and projection on every device.
2. A local edit is an atomic transaction, durably reserved before signing and
   committed before publication. Retry and restart reproduce the same bytes.
3. Provider order, timestamps, ETags, duplicate delivery, omission and denial
   never grant authority or silently delete history.
4. Every author is an exactly admitted, root-certified device. Readers cannot
   mutate. Each membership transition rotates the epoch key and closes history
   explicitly; excluded offline edits require review and explicit re-authoring.
5. Missing history pauses publication; authenticated forks freeze the space and
   preserve the last readable view and evidence.
6. An intact independent witness prevents copied or rolled-back profiles from
   silently reusing signing state. Restoring both profile and witness requires
   independently authenticated peer history or fresh-device admission.
7. Discovery, decoding and replay have explicit CPU, memory and request bounds;
   large valid concurrency must have a bounded path back to writable state.
8. Invitation, admission, device replacement and recovery authenticate exact
   identities and anchors without copying private device keys.

## Implementation and evidence

| Work | Acceptance evidence | Status |
| --- | --- | --- |
| Canonical signed events, causal transactions and whole-record conflicts | Byte fixtures, permutations, role and signature substitution tests | Implemented for experimental profile |
| Durable intent, outbox and incoming candidate commits | Restart, disk failure, acknowledgement and revocation tests | Implemented; additional fault injection required |
| Dependency discovery independent of LIST visibility | Pinned genesis and recursively fetched omitted ancestors | Implemented; bounded GET tests pass |
| Wide frontier folding | 66 independent device histories preserve conflicts, fold and resolve; Vault retries through durable merge intents | Implemented; crash/retry tests pass |
| Replay rejection and readiness | Invalid descendants cannot masquerade as missing history; all delivery orders agree | Implemented; invalid closed history freezes as a checkpoint fault |
| Installation and rollback protection | Backup rollback, missing binding, witness/intent crash states, lost acknowledgements, kernel leases across processes | Implemented; Keychain OS probe and fault-injection tests pass; full-machine restore remains an explicit trust boundary |
| Fresh-device identity recovery | Independent current control pin, fresh certified keys, current checkpoint after new admission, original backup kept readable | API implemented and tested; UI and lost-controller recovery pending |
| Addressed invitations and device lifecycle | Signed consent, cancellation, replay/idempotency, role checks, recovery and UI flows | Profile-1 core, durable Vault API and desktop sharing/member screens implemented; file, session, trust-code and rendering tests pass; full recovery wizard pending |
| Bounded large-history operation | Profiled maximum supported history, safe checkpoint/retention behavior and documented capacity | Planned |
| S3 exchange | Authenticated PUT/GET/LIST with independent clients, failures and pagination | Loopback LIST/PUT/GET; reusable live probe added; real service pending |
| Protocol release and security review | Stable wire schema, conformance corpus, dependency review and independent cryptographic review | Pending |

## Validation policy

Run formatting, strict Clippy, tests and doctests after each coherent change.
Add behavioral tests for new safety boundaries rather than tests that duplicate
implementation. Record actual results, including the exact limitations of local
HTTP fixtures. A live test must use fictional records, a fresh random namespace
and independently provisioned credentials; secrets never enter logs or fixtures.

S3 discovery follows immutable references as well as opaque ListObjectsV2
pagination. Neither ordering nor a complete listing is a cryptographic proof.
Provider details are checked against the primary
[AWS API contract](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjectsV2.html)
and [Storj compatibility notes](https://storj.dev/dcs/api/s3/s3-compatibility).

Release readiness requires completed implementation evidence and a live Storj
run. Independent review remains a separate release gate; self-review and passing
tests must not be presented as an independent audit.

## Validation snapshot: 2026-11-01

- `cargo test --locked --offline --all-targets --quiet`: 232 library tests and one
  public-API acceptance test passed; the explicit OS Keychain probe is ignored
  by the ordinary suite.
- `cargo test --locked --offline --doc --quiet`: two doctests passed.
- `cargo clippy --locked --offline --all-targets -- -D warnings`: passed.
- Formatting, diff whitespace checks and private Rustdoc link checks passed.
- The live probe's missing-configuration guard was exercised; it exited before
  network I/O. No live Storj result is claimed.
- `cargo run --locked --offline --example shared_space --quiet`: independent
  profiles completed signed admission, concurrent edits, retries and resolution.
- Desktop sharing evidence covers application state transitions, independent
  trust-code checks, scoped file workers and rendered components. A manual native
  desktop handshake against real Storj remains a release check.

Independent profile protection and fresh-device identity recovery are implemented
in [profile-protection.md](profile-protection.md). Next implementation priority is
the identity-recovery wizard and explicit controller recovery/transfer rules.
Signed invitations, consent, durable issuance policy, receipt-based admission and
desktop member management have a [documented workflow](sync-invitations.md).
