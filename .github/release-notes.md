Native packages include the desktop app and native browser host. Browser companion ZIPs are separate assets. Verify downloads using `SHA256SUMS`.

- macOS: choose `aarch64-apple-darwin` for Apple Silicon or `x86_64-apple-darwin` for Intel. Extract the ZIP and move `Lockbox.app` into Applications. These builds are ad-hoc signed and are not Apple notarized.
- Linux: install the `.deb` with `sudo apt install ./lockbox_*_amd64.deb` to install runtime dependencies and register the menu entry and `lockbox://` handler. The tarball requires those dependencies to be installed separately.
- Browser integration: follow `ext/README.md` at this release tag. The host is inside `Lockbox.app/Contents/MacOS/` on macOS and `/usr/bin/` in the Linux package. The Firefox ZIP is unsigned and supports temporary development installation.

Lockbox remains experimental; see the repository README for development status and security limits.
