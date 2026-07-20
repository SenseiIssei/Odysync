import { copyFileSync, readdirSync, existsSync } from "fs";
import { join } from "path";
import { homedir } from "os";

const bundleDir = join(process.cwd(), "..", "..", "target", "release", "bundle", "nsis");
const desktopDir = join(homedir(), "Desktop");

if (!existsSync(bundleDir)) {
  console.log("[copy-to-desktop] NSIS bundle directory not found, skipping.");
  process.exit(0);
}

const setupFiles = readdirSync(bundleDir).filter((f) => f.endsWith("-setup.exe"));

if (setupFiles.length === 0) {
  console.log("[copy-to-desktop] No setup exe found in bundle directory.");
  process.exit(0);
}

for (const file of setupFiles) {
  const src = join(bundleDir, file);
  const dst = join(desktopDir, file);
  copyFileSync(src, dst);
  console.log(`[copy-to-desktop] Copied ${file} to Desktop`);
}
