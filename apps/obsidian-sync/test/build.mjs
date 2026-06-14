// Bundle every test/*.test.ts into test/.bundled/ with the `obsidian` import
// aliased to a runtime stub, so `node --test test/.bundled` can run them.
// Uses esbuild, which is already a devDependency for the plugin build.

import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { readdirSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const entries = readdirSync(here)
  .filter((file) => file.endsWith(".test.ts"))
  .map((file) => resolve(here, file));

if (entries.length === 0) {
  console.error("No *.test.ts files found in test/.");
  process.exit(1);
}

await build({
  entryPoints: entries,
  outdir: resolve(here, "bundled"),
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node20",
  alias: { obsidian: resolve(here, "obsidian-stub.ts") },
  logLevel: "warning",
});
