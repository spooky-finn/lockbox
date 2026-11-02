# Developer guide

Run commands from the repository root with a Rust toolchain supporting edition 2024.
The primary desktop platform is macOS. The UI uses the system WebView;
Touch ID and Keychain integration live in platform adapters.
See [Touch ID storage](touch-id.md) for biometric protection and migration from
older builds. macOS builds require Xcode Command Line Tools, including Swift;
Touch ID development builds need no Apple signing credentials.

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

Use feature names as the first navigation step:

| Change | Location |
| --- | --- |
| Browser companion UI, manifests and form filling | `ext/` |
| Native messaging relay, callback protocol and browser origin policy | `src/browser/`, `src/bin/lockbox-browser-host.rs` |
| Create/unlock a vault, recovery words, access screens | `src/access/` |
| Desktop deep links and browser account consent | `src/desktop/browser.rs`, `src/desktop/window.rs` |
| Records, folders, editor, imports, history and record views | `src/records/` |
| Invitation files, joining, member workflows and sharing views | `src/sharing/` |
| Signed protocol and deterministic replay | `src/sync/protocol/` |
| Exchange phases, retries and background scheduling | `src/sync/scheduler.rs` |
| Transport contracts, discovery and Storj | `src/sync/transport/` |
| Encrypted file formats, profile persistence, recovery and witnesses | `src/vault/` |
| Preferences, config files, language selection and settings view | `src/settings/` |
| Session/workspace coordination, root view, runtime and windows | `src/desktop/` |
| Reusable widgets, icons and translations | `src/desktop/widgets/`, `icons.rs`, `i18n.rs` |
| Clipboard, Touch ID, autolock and cancellable jobs | `src/platform/` |

Keep models, protocol rules and framework-independent workflows free of Dioxus
and OS calls even though they share a feature folder with views and adapters.
Module dependencies preserve these boundaries; folders no longer represent
architectural layers. `desktop/workspace.rs` coordinates record, sharing, settings
and sync state. Record-specific operations remain in `records/`.

Native file dialogs for record imports and invitation exchange live beside their
features in `records/dialogs.rs` and `sharing/dialogs.rs`. Global runtime code owns
session lifecycle and worker result application. New providers implement the
existing `Storage` / `StorageFactory` contracts; local and shared vault behavior
continues through the `Vault` facade.

Public feature APIs are declared in each `mod.rs`. Internal paths match disk
locations, such as `records::editor`, `sharing::actions`, and `sync::transport`.
`src/compatibility.rs` preserves established `core`, `app`, `infra`, and `ui`
public imports without retaining those source directories. New code should use
feature paths. Unit tests stay beside their owning feature; `tests/` contains
public acceptance scenarios.

`Vault` owns record and folder mutations. Read collections through `entries()`
and `folders()`; write through `upsert_entry`, `delete_entry`, `save_folder`,
`delete_folder`, `append_records` and `restore_record`. Direct field access is
restricted to vault persistence. Commands own rollback and preserve retained shared
signing intents; views and workflow callers must not stage collections or implement undo.
Local entry edits retain only the changed records for rollback. Shared edits
retain both readable collections because recovery can replace the entire view.

UI handlers call feature workflows for participant aliases, conflict resolution and
recovery retries. `profile_summary`, `participant_name` and `participant_alias`
expose display metadata without handing components the private profile container.
Typed sync status is translated in `desktop/i18n.rs`.

Generic cancellable jobs live in `platform/jobs.rs`. Favicon HTTP/cache behavior
lives in `records/favicon.rs`, and default profile path policy in `vault/paths.rs`.
The UI owns rendering, native dialogs and session-bound result application.

## Maintenance notes

Sidebar: drag right divider; arrows resize 10 px, Shift+arrows 40 px.
Home/End select limits; double-click restores default. Width stays within
200–480 px, reserves 320 px for detail. Mobile hides divider.
Webview owns pointer capture; component unmount removes listeners and observer.

English is the default interface language when no preference is saved or a language
code is unknown. Preserve explicit preferences, including Russian.

Appearance/language: Settings only. System/light/dark theme + custom accent/background;
apply immediately, persist in local config, survive locking/restart.
System remains default; colors reset independently. Custom background covers workspace,
controls and lock screen; native backdrop matches. Background brightness determines
readable text/native appearance; resetting background restores saved theme mode.
Accent fills keep chosen RGB; text/focus adapt for actual surfaces.
Invalid saved color falls back alone.

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
