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

Follow the [responsive UI architecture](ui-architecture.md) for input paths,
reactive subscriptions, contrast, keyboard behavior and performance measurements.

## GitHub CI and releases

GitHub Actions runs [CI](../.github/workflows/ci.yml) for pushes to `main`, pull
requests, and merge queues. It also supports manual runs. Formatting, browser
checks, and release-script tests run first; strict Clippy, all-target Rust tests,
and doctests then run on Linux x86_64, macOS Apple Silicon, and Windows x64. Configure branch
protection to require **CI passed**. Documentation changes still receive this
check, so required checks cannot remain pending because of path filters.

The toolchain is pinned in `rust-toolchain.toml`. Native dependencies are installed
by a shared setup action. Rust dependencies are cached separately by build profile,
OS, architecture, toolchain, and dependency manifests; default-branch runs save
caches and PRs restore them. New commits cancel superseded CI runs. Disabling debug
symbols and incremental compilation reduces runner disk use; two compilation and
test workers bound memory usage. The browser companion has no npm dependencies,
so CI does not run `npm install`. Action revisions are pinned to commit SHAs and
Dependabot proposes grouped updates weekly.

To create a release:

1. Update `[package].version` in `Cargo.toml` and `version` in `ext/manifest.json`
   to the same `X.Y.Z`. Run `cargo check` to refresh the root package version in
   `Cargo.lock`, then `cargo check --locked`. Commit the version changes
   and merge them into the default branch. Browser ZIP filenames and the local
   macOS bundle use the manifest version automatically.
2. Push/merge to `main`. A version increase automatically starts the release
   workflow and publishes the completed release (not a draft). Pushes with an
   unchanged version only run CI; decreases fail validation. The initial push
   also releases the current version if its tag does not exist.
3. The workflow validates versions and requires an unused `vX.Y.Z` tag, reruns full
   CI at the selected commit, builds packages in parallel, smoke-tests each optimized
   browser host, verifies the complete asset set, and generates `SHA256SUMS`.
   The final job creates the tag/release at that exact commit, with generated
   changelog notes listing every commit since the highest reachable `vX.Y.Z`
   tag, including direct commits and merges. The first release includes all
   history. Existing tags and releases are never overwritten; automatic runs
   skip existing tags.

For recovery/retries or draft releases, use **Actions → Release → Run workflow**
on the default branch, or GitHub CLI (replace `OWNER/REPO`):

```sh
gh workflow run release.yml --repo OWNER/REPO --ref main \
  -f version=0.1.0 -F draft=true -F prerelease=false
```

Release assets include:

| Asset | Contents |
| --- | --- |
| `lockbox-X.Y.Z-aarch64-apple-darwin.zip` | Apple Silicon `Lockbox.app`, including native browser host |
| `lockbox-X.Y.Z-x86_64-apple-darwin.zip` | Intel `Lockbox.app`, including native browser host |
| `lockbox_X.Y.Z_amd64.deb` | Linux desktop app, host, runtime dependencies, menu and deep-link registration |
| `lockbox-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` | Linux binaries, notices, and runtime dependency instructions |
| `lockbox-X.Y.Z-x86_64-pc-windows-msvc.zip` | Windows x64 desktop executable and notices; requires WebView2 Evergreen Runtime |
| `lockbox-chrome-X.Y.Z.zip`, `lockbox-firefox-X.Y.Z.zip` | Browser companions |
| `SHA256SUMS` | SHA-256 hashes for all seven packages |

Linux builds use Ubuntu 22.04 to retain Linux Mint 21.1 compatibility. macOS builds
are ad-hoc signed without Apple credentials and are not notarized. Windows builds
are unsigned and do not yet support native browser integration. Firefox packages
are unsigned development ZIPs. Browser native-host registration remains a separate
step described in [the browser guide](../ext/README.md): packaged hosts are at
`/Applications/Lockbox.app/Contents/MacOS/lockbox-browser-host` on macOS and
`/usr/bin/lockbox-browser-host` after installing the Linux `.deb`.

Only the publication job receives `contents: write`; other jobs use read-only
repository access. The automatic `GITHUB_TOKEN` is sufficient, with no extra
secrets. Ensure repository Actions policy allows the pinned actions and release
creation, including any applicable tag rules. Push this repository to GitHub
(a GitLab remote alone cannot run GitHub Actions). The workflows currently watch
`main`, which must be the repository's default branch. Tag pushes do not trigger
another release. Releases at different commits run independently, so a later
unchanged-version push cannot cancel a queued version bump. Reruns at the same
commit are serialized. Release validation is also isolated from cancellation by
newer pushes. Intermediate artifacts expire after seven days and release assets
remain attached to the release.

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

Record editor: entries open in reading mode. Click a field to copy; double-click
to edit it. Typing only changes the draft. Save with the visible Save button or
Cmd/Ctrl+S (also on non-Latin layouts), or choose Save, Discard or Cancel when closing with Escape/the top-left close button or navigating away.
Validation or persistence failure retains the editor and pending destination.
Locking also attempts a final save, but never prevents a security lock; failed or invalid unsaved edits are discarded with a notice. An
untouched empty new record is not created. Password visibility and other editor
controls do not create revisions.

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
