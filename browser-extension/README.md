# Lockbox browser companion

Chrome and Firefox extensions for your existing encrypted `.lbx` vault. Suggests
matching accounts, fills a selected login, and generates, saves and fills new
passwords. The extension communicates with a local Rust native messaging host.
Vault encryption stays in the existing Rust implementation (Argon2id,
HKDF-SHA256, XChaCha20-Poly1305).

Firefox's install prompt declares authentication information and browsing activity
because usernames/passwords and the selected website's origin cross into the
local native companion. This is required even for local native messaging under
[Mozilla's disclosure rules](https://extensionworkshop.com/documentation/develop/best-practices-for-collecting-user-data-consents/).
No data is sent to a Lockbox server and there is no telemetry. Only the selected
website origin is passed to the host; browsing history is never read.

## Build and install

Requirements: Node.js 20+, the project's Rust/native desktop dependencies,
Chrome 120+ or Firefox 140+. No npm dependencies or package installation needed.
Commands below start at the repository root:

```sh
cargo build --locked --release --bin lockbox-browser-host
npm --prefix browser-extension run check
# Optional distribution archives (requires the zip command):
npm --prefix browser-extension run package
```

Chrome:

1. Open `chrome://extensions`, enable **Developer mode**, choose **Load unpacked**,
   and select `browser-extension/dist/chrome`.
2. Copy the extension's ID from that page (also shown in the popup's setup section).
3. Register the native companion, replacing both placeholders:

   ```sh
   node browser-extension/scripts/register-host.mjs \
     --browser chrome --extension-id YOUR_CHROME_EXTENSION_ID \
     --host /ABSOLUTE/PATH/TO/lockbox/target/release/lockbox-browser-host
   ```

Firefox:

1. Open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**,
   and select `browser-extension/dist/firefox/manifest.json`.
2. Register the companion:

   ```sh
   node browser-extension/scripts/register-host.mjs \
     --browser firefox \
     --host /ABSOLUTE/PATH/TO/lockbox/target/release/lockbox-browser-host
   ```

The registration script supports macOS and Linux; `--browser chromium` selects
Chromium's manifest directory. It grants access only to the specified Chrome ID
or `lockbox@lockbox.local` in Firefox. It refuses to overwrite existing manifests:
inspect and remove the old `app.lockbox.browser.json` explicitly before updating.
Keep the binary at the registered path and keep the Chrome unpacked directory
stable so its extension ID stays the same.

On Windows, produce the native manifest with `--output-dir`, then set the default
value of `HKCU\Software\Google\Chrome\NativeMessagingHosts\app.lockbox.browser`
or `HKCU\Software\Mozilla\NativeMessagingHosts\app.lockbox.browser` to its absolute
JSON path. Windows and Linux native builds have not been verified in this change.

For distribution, Chrome requires Web Store packaging/review; permanent Firefox
installation requires Mozilla signing. Development loading does not publish
anything. Never disable browser signing or security controls to install it.

## Use

Create a vault in the desktop app first. Lock that vault there before opening it
in the browser: the host takes the same exclusive file lease to prevent competing
writes. Each browser has its own session; lock one before using the other.

1. Open an HTTPS login page and open the Lockbox toolbar popup.
2. Enter the absolute `.lbx` path and master password. Neither is persisted by
   the extension. Unlocking uses the file's existing KDF settings.
3. Matching accounts appear by exact HTTPS origin, including effective port.
   Select **Fill**. The extension never submits the form.
4. For registration or password changes, expand **Suggest a new password** and
   generate 16–128 characters. Choose **New login** or an existing matching
   account. **Save, then fill new password** commits to the encrypted vault
   before filling. An existing account keeps its username, URL, notes, TOTP,
   attachments, folder and encrypted password history.
5. Submit the website form yourself. Saving in Lockbox does not change the
   account's password on the website. If the website rejects the new password,
   the previous saved password is available in desktop **Record history**.

Only visible fields in the top-level page are supported. For multiple forms,
focus the desired password field first. Cross-origin actions, GET forms,
iframes, hidden/disabled/read-only/offscreen fields, ambiguous password forms
and fields that would truncate the credential are refused. SPA forms without
a `<form>` element are supported when fields are unambiguous. Shadow DOM and
username-only first steps require manual entry.

The native session locks after five minutes without a successful non-status
request and after fifteen minutes regardless of activity, including time asleep.
Status polling cannot extend either deadline. **Lock vault**, browser idle/screen
lock events and native disconnects drop the native vault. OS idle-lock event
support varies by browser/platform; the native timers remain enforced. A stalled
or terminated browser background context causes reconnection to start locked.

Shared profiles use the same guarded local save/outbox behavior. This companion
does not run cloud synchronization; lock the browser session and open the
desktop app to exchange changes. It does not bypass installation or rollback
protection. Restoring/recovering vaults remains a desktop operation.

## Security boundary

- No browser storage permission, stored master password, exported vault key,
  local HTTP server, blanket website access, persistent content script, external
  messaging, clipboard access, remote scripts, analytics or favicon requests.
- `activeTab` grants temporary access after opening the toolbar popup. Only the
  exact internal popup can request native operations. Web pages/content scripts
  cannot request credentials or unlock the vault.
- Account suggestions omit passwords, notes, TOTP secrets and attachments.
  The selected credential crosses native messaging only after a fill click.
- The background checks the tab's HTTPS origin before lookup and again before
  fill. A one-use token in the isolated execution world binds fill to the exact
  inspected document and input elements. Navigation and unsafe form changes
  abort filling. All chosen values are set before website input handlers run.
- Native frames are bounded at 64 KiB, requests reject unknown fields/actions,
  and Rust wipes keys, entries, generated passwords and native message buffers
  when dropped. Malformed framing disconnects rather than continuing unlocked.
- **A website can read any password filled into it**, including through its own
  scripts. HTTPS/origin checks cannot make a compromised website trustworthy.
  Browser JavaScript, IPC and DOM copies cannot be guaranteed erased. A
  compromised OS, browser, another sufficiently privileged extension, screen
  capture, or local process running as your user is outside this boundary.
- Vault backups/history retain prior encrypted values. Generated replacements
  do not delete old encrypted passwords. There is no claim of independent audit
  or absolute security; obtain security review before treating this as production
  credential infrastructure.

Browser API references: [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging),
[temporary activeTab access](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab),
[Firefox native messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging),
[Firefox background scripts](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background).

## Verification

```sh
cargo test --locked browser::
cargo test --locked --test browser_host
npm --prefix browser-extension run check
```

Optional real Chromium form tests (uses a temporary profile, no real credentials):

```sh
CHROME_BIN="/path/to/chrome" npm --prefix browser-extension test
FIREFOX_BIN="/path/to/firefox" npm --prefix browser-extension test
```

The Chromium checks use a temporary loopback HTTPS fixture and require `openssl`.
The Firefox smoke test installs/uninstalls the built package in a disposable
profile through WebDriver BiDi. Neither test uses your normal browser profile.

To regenerate PNG browser icons from the existing brand vector:

```sh
cargo run --locked --bin build-browser-icons
```

Use fictional accounts for manual checks in both browsers. Check refused HTTP,
wrong origins/ports, navigation during fill, hidden fields, cross-origin form
actions, idle/screen locking, failed saves, registration and password changes.
