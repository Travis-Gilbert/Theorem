# theorem-acp

CommonPlace ACP host: subprocess JSON-RPC, harness MCP injection, scoped file
review, PTY command approval, and copresence registration.

## What it is

`theorem-acp` docks external ACP-capable coding agents into the CommonPlace
surface. It spawns the agent subprocess over newline-delimited JSON-RPC stdio,
advertises host-owned filesystem and terminal capabilities, injects the
Theorems-Harness V2 MCP server during `session/new`, and exposes frontend event
envelopes for native CommonPlace thread rendering.

It is not a terminal emulator. Agent command requests are staged as approval
cards, then executed through a backend PTY only after approval.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-acp
```

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in
[CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate.
