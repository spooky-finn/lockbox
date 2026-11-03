# Lockbox

**Local-first, serverless password manager for teams, powered by S3-compatible storage.**

Keep passwords, TOTP codes, and attachments in encrypted vaults on your own devices. Share access with your team through an object storage bucket, with local edits available offline and encrypted changes exchanged in the background.

Lockbox runs as a Rust desktop application built with Dioxus. Team synchronization uses object storage directly, so there is no dedicated Lockbox application server to deploy or maintain. The current storage integration targets Storj through its S3-compatible API; compatibility with other S3 providers has not been established.

> **Development status:** Lockbox is experimental and has not received an independent security audit. The shared sync protocol is unversioned and may change incompatibly. Live Storj bucket validation remains outstanding in the project documentation.

## What you can do

- **Keep a personal vault.** Store logins and TOTP secrets in one encrypted `.lbx` file. Work locally without configuring cloud storage.
- **Share with your team.** Invite participants, approve specific devices, assign roles, and revoke device access.
- **Work offline.** Save changes locally; a durable outbox retains shared changes for background synchronization.
- **Handle concurrent edits.** Review conflicting versions and resolve them explicitly.
- **Organize credentials.** Use folders, attach files, inspect record history, and restore entries from Trash.
- **Fill browser logins.** Chrome and Firefox companions request account selection and approval in the desktop app before filling an exact HTTPS origin.
- **Unlock with Touch ID.** Use macOS biometric access after setting it up with your master password.
- **Recover access.** New shared profiles provide 12 recovery words, with local and cloud recovery workflows.

English and Russian interfaces are available, along with system, light, and dark themes and custom colors.

## How team sync works

Each device maintains an encrypted local profile. Record and folder edits are saved locally before synchronization. Devices exchange encrypted, signed events through the configured bucket and verify signatures, membership, permissions, and causal dependencies before applying changes.

Invitations require participant verification and approval of a specific device. Each participant supplies their own bucket credentials; invitation links do not contain bucket credentials or vault keys. Membership changes rotate the encryption epoch key. Concurrent record versions remain available for review until resolved.

The bucket acts as a transport for encrypted data. Clients perform encryption, verification, and conflict handling. See the [sync engine guide](docs/sync-engine.md) for the implemented behavior and the [invitation guide](docs/sync-invitations.md) for joining and access management.

## Quick start

### Requirements

- macOS, the primary desktop platform.
- A Rust toolchain supporting edition 2024.
- Xcode Command Line Tools, including Swift.
- For shared vaults: a Storj bucket and S3 access credentials.

### Run from source

From your repository checkout:

```sh
cargo run --locked
```

For an optimized build:

```sh
cargo build --locked --release --bin lockbox
./target/release/lockbox
```

Browser integration requires an app bundle and a native messaging relay. Follow the separate [Chrome and Firefox setup guide](ext/README.md).

### Create your first vault

1. Open Lockbox and choose a local vault or shared mode.
2. For shared mode, enter your Storj connection details, user name, and vault name. Follow the [Storj setup guide](docs/storj-setup.en.txt).
3. Set a strong master password. Save the 12 recovery words provided for a new shared profile in a secure place.
4. Add your login records, TOTP secrets, and folders.
5. To share access, open **Participants and access**, create an invitation, and verify the participant through a separate channel before approving their device.

To explore synchronization without a cloud bucket, run the example with two fictional clients and an in-memory storage provider:

```sh
cargo run --locked --example shared_space
```

## Security and recovery

Local vaults use Argon2id, HKDF-SHA256, and XChaCha20-Poly1305. The default password derivation cost is 256 MiB of memory and three passes. Saves are atomic and preserve the previous file as `.lbx.bak`. Shared events are encrypted and signed.

Lockbox clears Rust secret buffers when locking and removes secret-bearing views. Copies in WebView memory, browser JavaScript, IPC, or the DOM cannot be guaranteed erased. Automatic locking defaults to five minutes of inactivity. Copied secrets are cleared after 20 seconds if the clipboard still contains the same value.

Keep backups of your vault and store recovery words separately. Cloud recovery requires recovery words and bucket access, and restores only published data. After losing all independent copies, you must explicitly trust the available cloud state; its freshness cannot be proven.

A few storage boundaries matter when using Lockbox:

- Record history and Trash retain previous values inside encrypted storage. There is no permanent purge operation.
- Downloading website icons is optional. Downloaded icons remain in an unencrypted local cache.
- The browser companion keeps an unencrypted local index of saved HTTPS origins, which can reveal which sites have saved logins.
- Exported attachments are ordinary files on disk.
- Updated vault formats may be rejected by older clients. Update devices before reopening upgraded files.

Read the [detailed usage guide](docs/usage.md), [profile protection guide](docs/profile-protection.md), and [browser security boundary](ext/README.md#security-boundary) for limits and recovery details.

## Platform support

macOS has the primary documented desktop workflow, including Touch ID and Keychain integration. Other platforms do not have equivalent documented integration coverage. The supplied browser deep-link installer is macOS-only; the browser bridge does not implement Windows support.

## Documentation

| Topic | Guide |
| --- | --- |
| All project documentation | [Documentation index](docs/README.md) |
| Build commands and code structure | [Developer guide](docs/development.md) |
| Browser installation and autofill | [Browser companion](ext/README.md) |
| Storage configuration | [Storj setup](docs/storj-setup.en.txt) |
| Implemented synchronization | [Sync engine](docs/sync-engine.md) |
| Team invitations and access | [Invitations](docs/sync-invitations.md) |
| Recovery words and encrypted profiles | [Recovery words](docs/recovery-words.md) |
| Recovery without the original vault | [Cloud recovery](docs/cloud-recovery.txt) |
| Biometric access | [Touch ID](docs/touch-id.md) |
| Release readiness | [Production plan](docs/sync-production-plan.md) |

Some detailed documents are in Russian. The [protocol proposal](docs/lockbox-protocol.md) describes a broader target design; use the sync engine guide for implemented capabilities.

## Contributing

Start with the [developer guide](docs/development.md). Keep changes focused and explain the resulting behavior, validation, and any security or compatibility implications. Use fictional credentials and a separate vault for testing. Keep passwords, recovery words, keys, and vault contents out of logs, issues, and fixtures.

Run the Rust checks from the repository root:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

For browser companion changes, also run:

```sh
npm --prefix ext run check
```

See [browser verification](ext/README.md#verification) for optional real-browser tests. Persistence and synchronization changes should cover failure and recovery paths as well as successful operations.

## License

A project-wide license has not yet been added. An open-source license must be selected before distributing Lockbox as licensed open-source software.

Bundled Simple Icons assets carry their own [license](assets/LICENSE-simple-icons.md) and [disclaimer](assets/DISCLAIMER-simple-icons.md).
