# theorem-agentd

`theorem-agentd` is the local face for Theorem. It wraps an
OpenAI-compatible local model, dispatches schema-guarded tool calls through MCP,
writes one JSONL ledger line per turn, and can embed `theorem-receiver` on a
dedicated OS thread.

The descriptive local-model binary is `theorem-localmodel`; `theorem-agentd`
remains as the compatibility binary. The proxy-only binary is `rustyred-proxy`
and reuses the same proxy module without starting the local model loop.

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

## Anthropic Messages proxy

`rustyred-proxy` starts a local Anthropic Messages-compatible endpoint:

```bash
cargo run -p rustyred-proxy -- \
  --proxy-port 8484 \
  --proxy-data-dir "/Volumes/SSD Samsung/theorem-local-proxy"
```

The compatibility command `theorem-agentd --proxy` serves the same endpoint.

The proxy serves `POST /v1/messages` on loopback and forwards Anthropic auth and
beta headers to the selected provider. With resident capabilities enabled, it
injects hidden cache-stable harness tools, consumes resident `tool_use` blocks
locally, appends `tool_result` blocks back into context, and returns only the
final assistant turn to the client. It exposes the selected
`compute_offload.route_operation` affordance from `rustyred-thg-affordances`
directly, plus the `tool_search`/`describe`/`invoke` gateway tools for affordance
discovery and execution. Disable this loop with
`THEOREM_PROXY_RESIDENT_CAPABILITIES=0` to restore byte-stream passthrough.

Resident tier-two and tier-three `invoke` calls are held when their input carries
`action_tier` without `human_authorized=true`; the proxy returns an
`approval_required` tool result with `executed=false` rather than running the
affordance. Oversized native `tool_result` blocks are sampled before forwarding;
error/anomaly rows survive, the `tool_use_id` is preserved, and the original
bytes are available from `GET /v1/tool-result-fetch?fetch_handle=...`.

The resident cascade is gated by corpus calibration. Set
`THEOREM_PROXY_LOCAL_ANTHROPIC_UPSTREAM` to an Anthropic-compatible local model
endpoint and `THEOREM_PROXY_CASCADE_CALIBRATION` to a JSON file shaped as:

```json
{
  "source": "SPEC-BEHAVIOR-CORPUS.md deliverables 3 and 5",
  "quality_floor": 0.72,
  "samples": [
    { "raw_score": 0.2, "observed_success": false, "weight": 1.0 },
    { "raw_score": 0.9, "observed_success": true, "weight": 1.0 }
  ]
}
```

If the calibration file is missing, unreadable, invalid, empty, or not sourced
from the behavior corpus deliverables, the proxy routes upstream.

Advisory verification reads `<proxy-data-dir>/verification_claims.json` or
`THEOREM_PROXY_VERIFICATION_CLAIMS`. Claims are JSON objects with `claim`,
`contradicted_by`, `basis`, and optional `checker`; matching assistant text
causes the proxy to inject a non-blocking verification advisory and ask the model
to revise instead of failing the turn.

The same proxy exposes the simplified local co-presence protocol from the
CommonPlace runtime:

```bash
curl -fsS -X POST http://127.0.0.1:8484/v1/presence \
  -H 'content-type: application/json' \
  --data '{"actor":"codex","path":"src/lib.rs","line":1,"col":0,"label":"Codex"}'
```

Use `POST /v1/presence/footprint` to publish a pending edit range and
`POST /v1/presence/would-overlap` to ask whether a peer has an overlapping
pending edit on the same path. The proxy keeps this registry local-first, so
Claude Code, Codex, Gemma, and other local clients can coordinate even when the
remote harness room is offline. The legacy `/v1/agents/*` room endpoints remain
available for compatibility.

Ambient context injection is opt-in by environment or harness availability. A
local file at `<proxy-data-dir>/ambient.md` or `THEOREM_PROXY_AMBIENT_TEXT`
injects into the latest user turn. When the harness MCP endpoint is reachable,
the proxy also attempts a bounded `hippo_retrieve` call using either the
desktop-provided harness bearer or the configured harness token environment
variable. Anthropic credentials are never used for harness calls.

## Use through the `theorem` wrapper

The repo-level wrapper gives the install/onboarding path a single command:

```bash
scripts/theorem init
scripts/theorem once "hello from the wrapper"
scripts/theorem harness
scripts/theorem start
scripts/theorem proxy --proxy-data-dir "/Volumes/SSD Samsung/theorem-local-proxy"
scripts/theorem appear codex
scripts/theorem codex-endpoint
scripts/theorem wrap claude
scripts/theorem wrap codex
```

`scripts/install.sh` installs that wrapper as `theorem` and can start the local
daemon with the same no-config defaults. It also installs `rustyred` as an alias
for the same wrapper, so `rustyred proxy` and `rustyred wrap claude` work after
install. For release installs, the wrapper prefers installed `rustyred-proxy`
and `theorem-localmodel` binaries and falls back to Cargo only when no binary
exists.

`theorem appear`, `theorem codex-endpoint`, and `theorem wrap <claude|codex>`
announce native local co-presence over `POST /v1/presence` and keep a short-TTL
heartbeat alive while the local process is alive. Set
`THEOREM_PROXY_LEGACY_ROOM_SYNC=1` to also best-effort mirror presence to the
older `/v1/agents/presence` harness-room compatibility endpoint.

`rustyred proxy` runs CPU-only by default and does not download a model or ONNX
asset on first launch. `THEOREM_PROXY_DATA_DIR` or `--proxy-data-dir` can point
the proxy state at an external volume.

## Tool-call guardrail

The tool catalog is the single source for:

- system prompt tool documentation
- the GBNF grammar closed over known tool names
- runtime tool-name and required-argument validation
- MCP server routing

The grammar prevents malformed JSON envelopes and hallucinated tool names. The
runtime validator rejects missing required arguments before MCP dispatch.
