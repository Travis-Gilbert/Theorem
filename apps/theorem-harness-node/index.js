// Entry point for the @theorem/harness-node package: loads the native addon.
// Build it first with `npm run build:debug` (debug) or `npm run build` (release),
// which compiles the Rust binding and copies the dylib to theorem_harness_node.node.
module.exports = require("./theorem_harness_node.node");
