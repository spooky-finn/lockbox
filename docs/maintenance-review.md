# Maintenance review — 2026-11-01

Six scoped maintenance items implemented. Existing Storj SDK and membership edits
preserved. Public facade imports, encrypted formats and canonical wire fixture
unchanged. No measured latency or memory improvement claimed.

## 1. Shared connection validation

[`infra/provider.rs`](../src/infra/provider.rs) owns pure connection syntax checks
and typed field errors. [`CloudSetup`](../src/app/cloud_setup.rs) maps errors to
existing form messages; [`Storj`](../src/infra/storj.rs) maps to transport errors.
Username/space-name checks remain in app; signing-region checks remain in adapter.
No network access during form validation; errors retain no supplied secrets.

Coverage: 31 form/transport parity cases, including URL whitespace/control,
embedded credentials, bucket bounds and credential syntax; explicit region guard.

## 2. Smaller local entry rollback

[`Unlocked`](../src/app/workspace.rs) retains previous entry for update, inserted
index for creation, removed entries and original indices for deletion. Local
operations no longer clone entire entry collection, including unrelated passwords
and attachment bytes.

Shared saves retain full snapshot: intent recovery can replace collection before
later persistence failure. Narrow undo would lose previously visible data.
[`Vault::save_changes`](../src/infra/vault/shared.rs) journal-copy reduction and
explicit Vault edit commands remain future investigation.

Coverage: failed create/update/delete preserve draft, selection, order, confirmation
and attachments; successful retry persists edit. Shared retry retains unsigned
intent and commits exactly one additional event.

## 3. Explicit replay stages

[`replay.rs`](../src/core/sync/machine/replay.rs) separates authenticated change
decoding, signed epoch closure, sequence-fork checks and causal application.
Control history verification lives in
[`history.rs`](../src/core/sync/machine/history.rs). Verification order and
rejected/pending/excluded/fault classifications preserved.

Coverage: delivery permutations, reader forgery, forks, invalid ancestry, checkpoint
faults and wide-frontier behavior. Canonical wire fixture unchanged.

## 4. Vault format and filesystem separation

[`vault.rs`](../src/infra/vault.rs) retains public facade and explicit generation/
witness ordering. Private [`format.rs`](../src/infra/vault/format.rs) owns header
codec, serialization, key derivation and authenticated encryption.
[`file.rs`](../src/infra/vault/file.rs) owns private files, process locks, backups
and atomic replacement. Both create and replace reuse one witness-commit tail.

Coverage: legacy formats, header tampering, password/key failures, backup behavior,
process leases, symlink protection and file/witness failure recovery. No generic
writer shared with config; durability contracts remain explicit.

## 5. Sync tests grouped by behavior

[`core/sync/tests/`](../src/core/sync/tests/mod.rs) contains focused `wire`,
`membership`, `replay` and `intents` modules plus deterministic shared support.
Original 21 tests and assertions retained. Public acceptance scenario remains in
`tests/`; canonical fixture not regenerated.

## 6. Documentation ownership

[Documentation index](README.md) separates current behavior, broader proposal and
historical plans. Stale format claims corrected; developer guide includes build,
run and icon-generation commands.

Detailed module ownership table lives in [architecture guide](sync-architecture.md);
[engine guide](sync-engine.md) links there and focuses on behavior, API and limits.
Initial checklist retained as history; remaining release work tracked only in
[production plan](sync-production-plan.md).

## Validation

Full offline suite passed with local socket access for S3 fixtures: 211 library
tests, one public acceptance test, two doctests. One explicit Keychain OS probe
ignored. Strict all-target Clippy, formatting, private Rustdoc links and edited
Markdown relative links passed. Two agents reviewed integrated changes without
finding actionable issues. No live Storj run performed.
