# theorem-agentd

`theorem-agentd` is the local face for Theorem. It wraps an
OpenAI-compatible local model, dispatches schema-guarded tool calls through MCP,
writes one JSONL ledger line per turn, and can embed `theorem-receiver` on a
dedicated OS thread.

It does not spawn Claude or Codex directly. When the model wants another head, it
uses the room tools (`coordinate` or `job_submit`), and the existing receiver
does the local launch.

## Run one turn

```bash
cp crates/theorem-agentd/theorem-agentd.example.toml ./theorem-agentd.toml
THEOREM_HARNESS_TOKEN=<bearer> \
  cargo run -p theorem-agentd -- --once "what are the agents working on" ./theorem-agentd.toml
```

The command prints a full JSON transcript: context reads, model decision, tool
calls, tool results, final text, and ledger status.
Single-turn mode does not start the receiver sidecar; daemon mode does.

For offline development, set `[model].provider = "rule"` in a copy of the config.
That provider is deterministic and only exists to test the daemon loop without a
resident model.

## Run as a daemon

```bash
THEOREM_HARNESS_TOKEN=<bearer> \
  cargo run -p theorem-agentd -- ./theorem-agentd.toml
```

If `[receiver].enabled = true`, startup also loads `theorem-receiver.toml` and
starts `theorem_receiver::run_loop` on its own OS thread. The model loop and the
receiver communicate only through the coordination room.

`theorem-agentd.launchd.plist.example` shows the single-job launchd shape. Do
not store real tokens in the plist; set them through launchctl or another secret
manager.

## Tool-call guardrail

The tool catalog is the single source for:

- system prompt tool documentation
- the GBNF grammar closed over known tool names
- runtime tool-name and required-argument validation
- MCP server routing

The grammar prevents malformed JSON envelopes and hallucinated tool names. The
runtime validator rejects missing required arguments before MCP dispatch.
