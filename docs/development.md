# Developer guide

Run commands from `scripts/lockbox` with a Rust toolchain supporting edition 2024.
The primary desktop platform is macOS. The UI uses the system WebView;
Touch ID and Keychain integration live in platform adapters.

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

## Maintenance notes

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
`assets/icon_domains.txt` and regenerate the catalog with `build-icons` as described
in the README rather than editing the large generated catalog manually.
