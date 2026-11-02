// Build a local macOS application bundle that owns lockbox:// links.
import { mkdir, copyFile, writeFile, stat, rename } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { spawnSync } from "node:child_process";
if (process.platform !== "darwin") throw new Error("This installer currently supports macOS only.");
const root = fileURLToPath(new URL("../../", import.meta.url));
const executable = path.join(root, "target/release/lockbox");
await stat(executable);
const bundle = path.join(root, "target/Lockbox.app");
const contents = path.join(bundle, "Contents");
await mkdir(path.join(contents, "MacOS"), { recursive: true });
const stagedExecutable = path.join(contents, "MacOS", `lockbox-${process.pid}.tmp`);
await copyFile(executable, stagedExecutable);
await rename(stagedExecutable, path.join(contents, "MacOS/lockbox"));
await writeFile(path.join(contents, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>app.lockbox.desktop</string>
<key>CFBundleName</key><string>Lockbox</string>
<key>CFBundleExecutable</key><string>lockbox</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>app.lockbox.browser</string>
<key>CFBundleURLSchemes</key><array><string>lockbox</string></array>
<key>CFBundleTypeRole</key><string>Viewer</string>
</dict></array>
</dict></plist>
`);
const registration = spawnSync("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister", ["-f", bundle], { stdio: "inherit" });
if (registration.status !== 0) throw new Error("Could not register lockbox:// handler.");
console.log(`Desktop deep links registered: ${bundle}`);
