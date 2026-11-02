import { execFileSync } from "node:child_process";
import { rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("../dist/", import.meta.url));
for (const browser of ["chrome", "firefox"]) {
  const destination = join(root, `lockbox-${browser}-0.1.0.zip`);
  await rm(destination, { force: true });
  execFileSync("zip", ["-q", "-X", "-r", destination, "."], { cwd: join(root, browser), stdio: "inherit" });
  console.log(`Packaged ${destination}`);
}
