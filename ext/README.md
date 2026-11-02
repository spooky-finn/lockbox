# Lockbox browser companion

The extension is a proxy for the desktop app. It sends the selected HTTPS origin,
opens Lockbox through a `lockbox://browser/fill?request=…` deep link, and fills the
one login explicitly approved in Lockbox. Unlocking, Touch ID, vault files,
account selection and editing stay in the desktop app. The extension has no
master-password input, vault file access, password generator or vault session.

## Install on macOS

Requirements: Node.js 20+, Rust/native desktop build dependencies, Firefox 140+
or Chrome 120+. Start in the repository root:

```sh
cargo build --locked --release --bin lockbox --bin lockbox-browser-host
npm --prefix ext run check
node ext/scripts/setup-desktop.mjs
```

The last command creates `target/Lockbox.app` and registers its `lockbox://` URL
scheme with Launch Services. Keep the bundle at that path. Run the command again
after rebuilding the desktop binary, then quit/reopen the app. Open this bundle when using browser autofill;
a separate `cargo run` process does not receive the bundle's deep links and can
hold a competing vault lease. Lock/close the vault in that separate process first.

Firefox:

1. Open `about:debugging#/runtime/this-firefox`.
2. Remove an older Lockbox temporary add-on if it points to another directory.
3. Click **Load Temporary Add-on**, select `ext/dist/firefox/manifest.json`.
4. Register the thin native relay once:

   ```sh
   node ext/scripts/register-host.mjs --browser firefox \
     --host "$PWD/target/release/lockbox-browser-host"
   ```

5. Open an HTTPS login page. Click the extensions puzzle, then **Lockbox** and
   **Open Lockbox**. To keep its icon visible, pin Lockbox from the extensions menu.
6. Unlock in the desktop app using its normal password or Touch ID flow. The
   browser request shows the exact website. Click **Fill** beside an account.
7. Return to the browser and submit the filled form yourself.

Temporary Firefox add-ons disappear on browser restart. Load the manifest again.
After extension changes, rebuild and click **Reload** in `about:debugging`.
Permanent Firefox distribution requires Mozilla signing.

Chrome:

1. Open `chrome://extensions`, enable **Developer mode**, click **Load unpacked**,
   select `ext/dist/chrome` and copy the extension ID.
2. Register the relay:

   ```sh
   node ext/scripts/register-host.mjs --browser chrome \
     --extension-id YOUR_CHROME_EXTENSION_ID \
     --host "$PWD/target/release/lockbox-browser-host"
   ```

3. Open an HTTPS login page and use **Open Lockbox** as above.

Registration refuses to overwrite an existing native manifest. If it already
points to the same host binary and extension ID, rebuilding the binary is enough.
Inspect the existing manifest before changing its path. The macOS Firefox manifest
is `~/Library/Application Support/Mozilla/NativeMessagingHosts/app.lockbox.browser.json`.

The native relay supports Unix sockets. The supplied deep-link installer is macOS
only; Linux requires separate desktop URL-handler registration. Windows support
is not implemented for this bridge.

## Automatic popup

The extension checks the active HTTPS origin against a local plaintext index,
even when Lockbox is closed or its vaults are locked. The relay returns only a
boolean; account names and passwords are not returned. Checks do not launch or
foreground the app. A match offers to open Lockbox; unlocking and explicit account
selection are still required before filling.

The index lives in `~/.lockbox/browser-sites/`, with one JSON file per vault.
Files contain only deduplicated HTTPS origins (scheme, hostname, effective port),
without paths, query strings, usernames or passwords. The directory is owner-only
(`0700`), files are `0600` on Unix. **The index is not encrypted: it reveals which
sites have saved passwords to anyone who can read these files.** It is local to
this device and is not synced or stored by the extension.

Unlock each existing vault once after updating to create its index. Successful
vault opens and saves refresh the index, including account deletion, URL changes
and synced changes saved locally. Locking or quitting preserves it. Vaults edited
elsewhere remain at their last indexed state until reopened locally. Removing a
vault file externally does not remove its index. To clear the index, quit Lockbox
and delete `~/.lockbox/browser-sites/`; opening a vault recreates its file. Index
write errors are logged and do not prevent saving the encrypted vault.

A match opens the popup once per loaded page. Firefox 149+ supports automatic
popup opening; older Firefox versions keep a **1** badge for manual opening.
The browser must be focused. Checks run on page load, tab/window activation and
every five seconds. No account index is persisted in the extension.

The extension uses the `tabs` permission to read the active website URL and sends
only its HTTPS origin to the local relay. Clicking **Open Lockbox** requests
optional access to that website for form filling, since an automatically opened
popup does not itself grant `activeTab`. Password filling still requires desktop
account selection and explicit consent.

## What to expect

The popup immediately displays **Lockbox extension loaded**. Native connection
errors appear below it. If clicking Open Lockbox does nothing, verify that the
native manifest is registered and rerun `node ext/scripts/setup-desktop.mjs`.
The popup can close while the app is in front; the background keeps the request.
Reopening it shows progress and a **Cancel request** button. Requests expire after
two minutes. Closing the browser connection or cancelling rejects pending consent.

No exact match: add/edit the login in Lockbox, or unlock the vault containing it.
Autofill uses exact HTTPS scheme/host/effective port. Navigation or changed forms
can block filling even after approval; retry from the intended login page.

Only visible fields in the top-level page are supported. For multiple forms,
focus the password field first. Cross-origin actions, GET forms, unsafe submit
method/action overrides, iframes, hidden/disabled/read-only/offscreen fields,
ambiguous forms and truncated values are refused. SPA forms are supported when
unambiguous. Shadow DOM and username-only steps require manual entry.

## Security boundary

- Deep links carry a random one-use identifier, never the website's password,
  master password, vault path or a callback URL. The website origin comes from
  the browser-owned native request through the local connection.
- The relay has no vault APIs. Old unlock/generate/credential requests are rejected.
- A private owner-only directory holds random Unix sockets. The desktop proves
  possession of the full request token before the relay discloses the origin.
  The reply goes back on that connection; no HTTP listener, browser URL callback,
  credential files, clipboard, cloud server or telemetry is involved.
- Unlocking does not approve autofill. The user selects an exact-origin account
  in the app. Approval rechecks the current unlocked vault and entry origin.
- The extension checks the active tab before and after desktop consent. The page
  script binds a one-use token to the inspected document and fields and rechecks
  form safety before filling. Navigation and unsafe changes abort the operation.
- Secrets are wiped on Rust buffer drop. Browser JS/IPC/DOM copies cannot be
  guaranteed erased. A website can read credentials filled into its own form.
- A compromised OS, browser or process running as the same user is outside this
  boundary. This implementation has not received an independent security audit.

## Verification

```sh
cargo test --locked browser:: --lib
cargo test --locked --test browser_host
npm --prefix ext run check
```

Optional real browser tests use disposable profiles and fictional credentials:

```sh
CHROME_BIN="/path/to/chrome" npm --prefix ext test
FIREFOX_BIN="/path/to/firefox" npm --prefix ext test
```

Manual end-to-end check: build/install both desktop and relay, open a test HTTPS
login page, request autofill, unlock with password or Touch ID, choose an account,
return to Firefox, verify filled fields. Repeat with cancellation, no match,
locked vault, browser restart and navigation before approval.

References: [Mozilla native messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging),
[Apple URL scheme registration](https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CoreFoundationKeys.html),
[Firefox temporary installation](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/).
