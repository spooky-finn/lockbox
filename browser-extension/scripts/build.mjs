import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const base = JSON.parse(await readFile(path.join(root, "manifest.json"), "utf8"));
for (const browser of ["chrome", "firefox"]) {
  const out = path.join(root, "dist", browser);
  await mkdir(out, { recursive: true });
  await cp(path.join(root, "src"), out, { recursive: true });
  await cp(path.join(root, "icons"), path.join(out, "icons"), { recursive: true });
  await cp(path.join(root, "../assets/lockbox-mark.svg"), path.join(out, "mark.svg"));
  const manifest = structuredClone(base);
  if (browser === "chrome") {
    manifest.minimum_chrome_version = "120";
    manifest.background = { service_worker: "background.js", type: "module" };
  } else {
    manifest.background = { scripts: ["background.js"], type: "module" };
    manifest.browser_specific_settings = {
      gecko: { id: "lockbox@lockbox.local", strict_min_version: "140.0",
        data_collection_permissions: { required: ["authenticationInfo", "browsingActivity"] } },
    };
  }
  await writeFile(path.join(out, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  await cp(path.join(root, "README.md"), path.join(out, "INSTALL.md"));
  console.log(`Built ${browser}: ${out}`);
}
