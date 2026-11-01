# Developer guide

Run commands from the repository root with a Rust toolchain supporting edition 2024.
The primary desktop platform is macOS. The UI uses the system WebView;
Touch ID and Keychain integration live in platform adapters.

```sh
cargo run --locked
cargo run --locked --example shared_space
```

The first command launches the desktop app; the second runs two fictional shared
profiles through an in-memory provider. See the [documentation guide](README.md)
for implemented sync behavior, release plans and the protocol proposal.

## Formatting and validation

```sh
cargo fmt --all
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Separate types, implementations, and functions with blank lines. Within a function,
separate distinct stages of work. Comments should explain constraints and reasons;
use `///` for public contracts and `//!` for module responsibilities.
Use English for new documentation and developer notes.

## Module boundaries

- `core`: domain models and rules, independent of Dioxus, storage, and transport.
- `app`: application workflows and state transitions, independent of Dioxus.
- `infra`: persistence, transport, clipboard, biometrics, and system adapters.
- `ui`: Dioxus components, contexts, localization, and window lifecycle.

Compatibility exports live in `src/lib.rs`. Preserve existing public import paths
when moving implementations between modules.

Record workflows live together in `src/app/records/`: workspace state, editor
drafts, folder navigation, tree projection and imports. Internal re-exports in
`app/mod.rs` preserve existing caller paths. Record views live in
`src/ui/components/records/`, while reusable controls live in
`src/ui/components/shared/`. Unit tests stay beside their owning feature;
`tests/` contains public acceptance scenarios.

`Vault` owns record and folder mutations. Read collections through `entries()`
and `folders()`; write through `upsert_entry`, `delete_entry`, `save_folder`,
`delete_folder`, `append_records` and `restore_record`. Direct field access is
restricted to infrastructure. Commands own rollback and preserve retained shared
signing intents; UI and app callers must not stage collections or implement undo.
Local entry edits retain only the changed records for rollback. Shared edits
retain both readable collections because recovery can replace the entire view.

UI handlers call app workflows for participant aliases, conflict resolution and
recovery retries. `profile_summary`, `participant_name` and `participant_alias`
expose display metadata without handing components the private profile container.
Typed sync status is translated in `ui/i18n.rs`.

Generic cancellable jobs live in `infra/jobs.rs`. Favicon HTTP/cache behavior
lives in `infra/favicon.rs`, and default profile path policy in `infra/paths.rs`.
The UI owns rendering, native dialogs and session-bound result application.

## Maintenance notes

Sidebar: drag right divider; arrows resize 10 px, Shift+arrows 40 px.
Home/End select limits; double-click restores default. Width stays within
200–480 px, reserves 320 px for detail. Mobile hides divider.
Webview owns pointer capture; component unmount removes listeners and observer.

English is the default interface language when no preference is saved or a language
code is unknown. Preserve explicit preferences, including Russian.

For persistence and sync changes, exercise failure and recovery paths as well as
successful operations. Local saves must work offline. See
[sync-engine.md](sync-engine.md) for the implemented synchronization profile;
[lockbox-protocol.md](lockbox-protocol.md) is an architectural proposal rather
than a list of implemented features.

Use a separate vault with fictional entries for manual UI checks. Check locking,
cancelled edits, and error messages. Keep passwords, keys, and vault contents out
of logs and fixtures.

Icons are bundled in the binary. Update domain mappings in
`assets/icon_domains.txt` rather than editing the large generated catalog manually.
With an unpacked `simple-icons` npm package available locally, regenerate it using:

```sh
cargo run --locked --bin build-icons -- /path/to/unpacked/simple-icons/package
```

Review the generated asset diff and retain upstream license/attribution assets.
The tool's module documentation describes the source archive layout.
