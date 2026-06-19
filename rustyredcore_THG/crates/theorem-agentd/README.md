# theorem-agentd

`theorem-agentd` is the local face for Theorem. It wraps an
OpenAI-compatible local model, dispatches schema-guarded tool calls through MCP,
writes one JSONL ledger line per turn, and can embed `theorem-receiver` on a
dedicated OS thread.

It does not spawn Claude or Codex directly. When the model wants another head, it
uses the room tools (`coordinate` or `job_submit`), and the existing receiver
does the local launch.

## Run one turn

No config file is required for the first smoke. If `theorem-agentd.toml` is
absent, the daemon uses safe local defaults: deterministic `rule` model, local
MCP route (`http://127.0.0.1:8380/mcp`), tenant `Travis-Gilbert`,
receiver/capture/relay off, and the ledger at `.theorem/agentd-token-ledger.jsonl`.

```bash
cd rustyredcore_THG
cargo run -p theorem-agentd -- --once "hello from agentd"
```

The command prints a full JSON transcript: context reads, model decision, tool
calls, tool results, final text, and ledger status. Single-turn mode does not
start the receiver sidecar; daemon mode does.

To pin settings explicitly:

```bash
cp crates/theorem-agentd/theorem-agentd.example.toml ./theorem-agentd.toml
cargo run -p theorem-agentd -- --once "hello from configured agentd" ./theorem-agentd.toml
```

Switch `[model].provider` to `openai-compatible` only when a resident model is
available. Keep model keys in environment variables, not in TOML.

## Resident Gemma vision serving

Image-bearing turns are sent through the same OpenAI-compatible
`/chat/completions` path as text turns. For Gemma 4 12B, llama-server must load
the multimodal projector or image parts are inert:

```bash
llama-server -m gemma-4-12b-it-q4.gguf --mmproj mmproj-gemma-4-12b-it.gguf -c <ctx>
```

If a prebuilt 12B projector is not available, convert it from the Hugging Face
checkpoint with llama.cpp's `convert_hf_to_gguf.py --mmproj`. This crate only
adds vision image input plumbing; audio projectors and screenshot capture are
separate follow-ups.

## Run as a daemon

```bash
cd rustyredcore_THG
cargo run -p theorem-agentd -- ./theorem-agentd.toml
```

If `[receiver].enabled = true`, startup also loads `theorem-receiver.toml` and
starts `theorem_receiver::run_loop` on its own OS thread. The model loop and the
receiver communicate only through the coordination room.

`theorem-agentd.launchd.plist.example` shows the single-job launchd shape. Do
not store real tokens in the plist; set them through launchctl or another secret
manager.

## Use through the `theorem` wrapper

The repo-level wrapper gives the install/onboarding path a single command:

```bash
scripts/theorem init
scripts/theorem once "hello from the wrapper"
scripts/theorem start
```

`scripts/install.sh` installs that wrapper as `theorem` and can start agentd
with the same no-config defaults.

## Tool-call guardrail

The tool catalog is the single source for:

- system prompt tool documentation
- the GBNF grammar closed over known tool names
- runtime tool-name and required-argument validation
- MCP server routing

The grammar prevents malformed JSON envelopes and hallucinated tool names. The
runtime validator rejects missing required arguments before MCP dispatch.
