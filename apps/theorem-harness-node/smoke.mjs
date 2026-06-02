// JS smoke test for the theorem-harness Node binding.
//
// Proves the round-trip JS -> Rust SDK -> GraphStore -> back to JS against the
// built .node addon. Build first:
//   cargo build --manifest-path apps/theorem-harness-node/Cargo.toml
//   cp apps/theorem-harness-node/target/debug/libtheorem_harness_node.dylib \
//      apps/theorem-harness-node/theorem_harness_node.node
//   node apps/theorem-harness-node/smoke.mjs

import { createRequire } from "module";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const { Harness } = require(join(here, "theorem_harness_node.node"));

const harness = new Harness();

const runId = harness.startRun("demo from node", "node-smoke", "k-create");
console.log("started run:", runId);

let events = JSON.parse(harness.eventsJson(runId));
console.log("after start:", events.length, events.map((e) => e.kind));

harness.cancel(runId, "stopping from node", "k-cancel");
events = JSON.parse(harness.eventsJson(runId));
console.log("after cancel:", events.length, events.map((e) => e.kind));

const text = harness.pollText(runId, 0);
console.log("text view:", JSON.stringify(text));

const ok =
  events.length === 2 &&
  events[0].kind === "Created" &&
  events[1].kind === "Cancelled" &&
  text.includes("stopping from node");

if (ok) {
  console.log("SMOKE PASS");
} else {
  console.error("SMOKE FAIL");
  process.exit(1);
}
