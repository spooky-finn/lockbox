# Dependency review — 2026-11-28

Scope: Dependabot PRs #1–#6 on `spooky-finn/lockbox`, applied to GitLab
`main` at `85793d6`, plus the vulnerable transitive cookie dependency.
These PRs include routine version updates, not just security fixes.

## Updates and compatibility

| Dependency | Previous | Updated | Review |
| --- | --- | --- | --- |
| argon2 | 0.5.3 | 0.6.0 | Rust 1.85 minimum is below the pinned 1.95 toolchain. Retain explicit Argon2id v1.3 parameters, caller-owned zeroizing arena and `zeroize`. Regression vectors captured with 0.5.3 verify the derived encryption key and password check for one and two lanes. |
| bip39 | 2.2.2 | 3.0.0 | Migrate deprecated `words_by_prefix` to `words_by_prefix_iter`, retaining the six-suggestion limit. OS entropy generation remains in Lockbox; the changed upstream generation API is unused. Existing BIP39/SLIP-0010 vectors and zeroize-on-drop checks protect recovery compatibility. |
| dirs | 6.0.0 | 7.0.0 | Source comparison changes Windows `preference_dir` from local to roaming AppData. Lockbox does not call this function; its home/config/cache/data paths are unchanged. Desktop dependencies still bring in dirs 6. |
| flate2 | 1.1.10 | 1.1.11 | Upstream stream flush/reset/error-handling fixes. Retain the Rust backend and existing bounded invitation decode checks. |
| url | 2.5.8 | 2.5.9 | Fixes RUSTSEC-2026-0592 (stack overflow resolving nested blob origins). Lockbox still requires HTTPS for browser origins. Add a nested-blob regression test. |
| cookie (via wry) | 0.18.2 | 0.18.3 | Fixes RUSTSEC-2026-0594 (private-cookie nonce reuse after fork). Upgrade even though exploitation requires private-cookie encryption across forked processes. |
| actions/setup-go | 6.5.0 | 7.1.0 | Retain immutable SHA pinning. Reviewed upstream action metadata: still Node 24; existing Go version and cache inputs remain supported. Hosted execution requires CI, not a local macOS test. |

Only these packages and necessary transitive dependencies were updated; other
cryptographic pins and the existing `phf_generator` patch remain intact.

## Security audit

`cargo audit 0.23.0`, RustSec database commit
`c5279d0b0c9e8b70df666eb6d1edbabcfd1176bd` (2026-11-27):

- Before: two vulnerability findings, `url` and `cookie` above.
- After: **zero vulnerability findings**, with four outstanding warnings.
- `glib 0.18.5`: **unsoundness warning RUSTSEC-2024-0429** in
  `VariantStrIter`. This remains in the Linux GTK3/WebKit dependency chain
  (`dioxus-desktop` → `wry`/`tao`/`gtk`). Fixed upstream in glib >=0.20;
  replacing this major version alone is incompatible with the GTK3 bindings.
  Requires a coordinated desktop-stack migration or a reviewed backport.
  This is not a clean bill of health for Linux.
- Unmaintained dependencies: `fxhash` (RUSTSEC-2025-0057), `paste`
  (RUSTSEC-2024-0436), and `proc-macro-error` (RUSTSEC-2024-0370).
  They remain transitive dependencies; no warnings were suppressed.

Sources: upstream crate changelogs/source downloaded by Cargo, the Dependabot
PR descriptions, [RustSec](https://rustsec.org/advisories/), and
[setup-go v7.1.0 metadata](https://github.com/actions/setup-go/blob/d8cbb8045d7f43706b9177cceb36c1f91a6644d5/action.yml).

## Validation results

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed, including
  both new regression tests.
- Pre-upgrade `cargo test --locked --all-targets`: 682 passed, 3 ignored.
- Upgraded `cargo test --locked --all-targets`: 683 passed, 3 ignored.
  The subsequently added nested-blob test also passed in a targeted run;
  the old-version password-key compatibility test passed both ways.
- `npm --prefix ext run check`: 26 passed, 1 skipped (real Firefox probe);
  Chrome and Firefox companion builds passed.
- `node --test tests/*.test.mjs`: 23 passed, 1 pre-existing failure below.
- Storj issuer `go vet ./...` and `go test -mod=readonly ./...`: passed.
- `cargo run --locked --example shared_space`: passed; independent profiles
  converged, concurrent versions survived, retries were idempotent, and
  explicit conflict resolution synchronized.
- Signed macOS release build and installation to `/Applications/Lockbox.app`:
  passed using `Lockbox Local Signing`; deep/strict signature verification
  passed. Packaged native host exited cleanly on EOF and rejected HTTP,
  embedded credentials, and deeply nested blob URLs without a credential reply.

## Validation limits

Automated tests cannot prove that dependencies are free from malicious code or
unknown vulnerabilities. Local verification covers macOS, not native Linux or
Windows execution, real browser installation, Touch ID, or live Storj access.

The pre-upgrade JavaScript baseline already fails
`tests/unlock-navigation.test.mjs` → “arrows respect other views, the vault
picker, and unavailable controls”: the script does not check `pickerOpen`.
This unrelated behavior is not changed by the dependency migration.
