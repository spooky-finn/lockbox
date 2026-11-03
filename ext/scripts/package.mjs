import { execFileSync } from "node:child_process";
import { readFile, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("../dist/", import.meta.url));
const { version } = JSON.parse(await readFile(new URL("../manifest.json", import.meta.url), "utf8"));
for (const browser of ["chrome", "firefox"]) {
  const destination = join(root, `lockbox-${browser}-${version}.zip`);
  await rm(destination, { force: true });
  execFileSync("zip", ["-q", "-X", "-r", destination, "."], { cwd: join(root, browser), stdio: "inherit" });
  console.log(`Packaged ${destination}`);
}
