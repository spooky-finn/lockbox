// Run explicitly after loading the extension. Never registers wildcard extension IDs.
import { mkdir, writeFile, stat } from "node:fs/promises";
import { homedir, platform } from "node:os";
import { resolve, join, isAbsolute } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: {
  browser: { type: "string" }, "extension-id": { type: "string" }, host: { type: "string" },
  "output-dir": { type: "string" },
} });
if (!["chrome", "chromium", "firefox"].includes(values.browser) || !values.host) {
  throw new Error("Usage: node scripts/register-host.mjs --browser chrome|chromium|firefox --host /absolute/path/lockbox-browser-host [--extension-id CHROME_ID] [--output-dir DIR]");
}
if (!isAbsolute(values.host)) throw new Error("Host path must be absolute.");
const host = resolve(values.host);
if (!(await stat(host)).isFile()) throw new Error("Native host binary not found.");
if (values.browser !== "firefox" && !/^[a-p]{32}$/.test(values["extension-id"] || "")) {
  throw new Error("Provide the exact 32-letter Chrome extension ID from chrome://extensions.");
}
let directory = values["output-dir"];
if (!directory) {
  const paths = platform() === "darwin" ? {
    chrome: "Library/Application Support/Google/Chrome/NativeMessagingHosts",
    chromium: "Library/Application Support/Chromium/NativeMessagingHosts",
    firefox: "Library/Application Support/Mozilla/NativeMessagingHosts",
  } : platform() === "linux" ? {
    chrome: ".config/google-chrome/NativeMessagingHosts",
    chromium: ".config/chromium/NativeMessagingHosts",
    firefox: ".mozilla/native-messaging-hosts",
  } : null;
  if (!paths) throw new Error("On Windows, use --output-dir and register the manifest path in the registry as described in README.md.");
  directory = join(homedir(), paths[values.browser]);
}
await mkdir(directory, { recursive: true, mode: 0o700 });
const manifest = {
  name: "app.lockbox.browser", description: "Lockbox encrypted vault companion", path: host, type: "stdio",
  ...(values.browser === "firefox" ? { allowed_extensions: ["lockbox@lockbox.local"] } :
    { allowed_origins: [`chrome-extension://${values["extension-id"]}/`] }),
};
const destination = join(directory, "app.lockbox.browser.json");
await writeFile(destination, JSON.stringify(manifest, null, 2) + "\n", { mode: 0o600, flag: "wx" });
console.log(`Registered ${values.browser}: ${destination}`);
