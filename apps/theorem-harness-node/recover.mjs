// Durability check: a FRESH process recovers a run from the RedCore AOF through
// the binding. Run after smoke.mjs against the same data dir:
//   DIR=$(mktemp -d)
//   RUNID=$(node smoke.mjs "$DIR" | grep '^RUNID=' | cut -d= -f2)
//   node recover.mjs "$DIR" "$RUNID"   # prints RECOVER PASS

import { createRequire } from "module";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const { Harness } = require(join(here, "theorem_harness_node.node"));

const dataDir = process.argv[2];
const runId = process.argv[3];
if (!dataDir || !runId) {
  console.error("usage: node recover.mjs <dataDir> <runId>");
  process.exit(2);
}

// A brand-new Harness over the SAME dir: RedCore recovers state from the AOF.
const harness = new Harness(dataDir);
const events = JSON.parse(harness.eventsJson(runId));
console.log("recovered events:", events.length, events.map((e) => e.kind));

const ok =
  events.length === 2 &&
  events[0].kind === "Created" &&
  events[1].kind === "Cancelled";

console.log(ok ? "RECOVER PASS" : "RECOVER FAIL");
process.exit(ok ? 0 : 1);
