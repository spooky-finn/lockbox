# Responsive desktop UI

## Boundaries

- Record data and persistence belong to `records` and the vault command API.
- `Draft::set_field` updates one zeroized input buffer. Do not clone an entry to
  detect a keystroke: entries can include large attachments.
- `Control` commands change only transient editor controls, never persisted data.
- Multi-field edits use `edit_draft` without cloning a comparison snapshot.
  Dirty checks compare the draft with the saved record only when leaving it.
- Typing never schedules persistence. Save explicitly, or confirm saving when
  closing or navigating away. Lock still attempts to flush the draft before
  clearing secrets. Failed navigation saves retain the draft and destination.
- Vault saves remain transactional. Do not bypass durable writes or weaken KDF
  parameters to make interaction appear faster.

## Reactive work

The app shell subscribes to memoized lock state and window title, rather than every
session edit. The sidebar first subscribes to the vault path and an in-memory
records revision.
Record commands, verified projection replacement and receive rollback invalidate
that revision. It is never serialized or used for signing or durable ordering.
Only record-version or normalized-query changes build the metadata equality key;
draft input does not scan entries. A second memo caches the sorted display
hierarchy, so selection and folder expansion do not repeat projection or sorting.
Search separately invalidates expansion behavior even if its result rows are equal.

The caches retain IDs, titles, folder names and search-match decisions only, never
passwords, notes, attachments, usernames or URLs. On lock they become empty. Folder
selectors cache their paths and exclusion rules by record version and excluded
folder; icon search runs only when its query changes. This is not virtualization.

The one-second clock is subscribed to by the TOTP row, not the complete entry card.
Copy indicators expire through scoped timers rather than clock-driven card renders.
Pending double-click cancellation uses nonreactive state; successful copies publish
clipboard status without notifying the session unless an error changes.

Regression tests use 1,000 records and 100 folders to verify that typing does not
scan metadata, project/sort the tree, or rebuild folder options. They also verify
that selecting and expanding reuse the hierarchy, saved changes refresh it, and
clock ticks do not render the complete entry card.

Native WebView navigation stays in `assets/tree-navigation.js`, with a workspace
listener disposed on sidebar unmount. Up/Down and Home/End change the selected
visible row without scrolling. Left/Right collapse, expand, or select a parent or
child. Tab focus alone does not select. Inputs, composition, modified keys, modal
dialogs and controls with their own arrow behavior retain native handling.

The WebView sends only row IDs and action kinds through its scoped evaluator.
Rust validates IDs, records activity and routes selection through `navigate`, so
unsaved edits retain Save/Discard/Cancel protection. Keyboard folder selection
preserves its disclosure state; explicit folder clicks still toggle it. Ignore
queued navigation while a confirmation is pending; never replace its destination.

## Appearance and feedback

Theme preferences are independent of vault contents. Presets supply coherent
light/dark surfaces; contrast-adjusted semantic ink must meet 4.5:1 on each
surface. Gradients interpolate checked surfaces, not arbitrary accent fills.
Legacy custom preferences load unchanged until a preset is explicitly chosen.

Creation feedback is a UI-only event emitted after a successful first save.
Validation failures, failed writes, subsequent saves and closing an untouched
draft must not celebrate. Use a bounded compositor-friendly animation, no flashes,
no input blocking, and honor reduced motion with a static status message.

## Validation and next measurements

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
node --test tests/tree-navigation.test.mjs
```

Use a fictional vault on each supported WebView for interactive validation.
Profile typing and button-to-paint latency separately with 100, 1,000 and 10,000
records, nested folders and large attachments. Record median and p95 values,
including release-build timings. Compare search, tree expansion, editor controls,
first save and subsequent saves. Never log real credentials or record contents.

Remaining work requires measurement: session-wide render notifications, very large
tree virtualization, synchronous encrypted saves after explicit user actions. Moving
saves to workers requires an explicit single-writer protocol, versioned snapshots,
failure retention, lock/cancel behavior and shutdown flushing; do not introduce an
uncoordinated background save or stale-snapshot overwrite.
