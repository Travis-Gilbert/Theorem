# theorem-proxy

A local Theorem node that sits on the model path. It speaks the Anthropic Messages
API on `localhost`, so a Claude Code (or any Anthropic-Messages client) session can
point `ANTHROPIC_BASE_URL` at it and run identically -- and the substrate can then
inject memory and coordination ambiently, with no MCP tool calls. This is the third
surface of the harness inversion (file watcher, phone control, now the model path):
capabilities become products of the environment, not tools the agent elects to call.

## Status

**Deliverable 1 shipped** (`SPEC-LOCAL-PROXY-MVP` D1): a faithful passthrough.
`POST /v1/messages` forwards to the upstream (default `https://api.anthropic.com`),
streaming and non-streaming, with headers, body, and the SSE event stream preserved
byte-for-byte -- nothing is parsed or mutated, so `tool_use` ids, the
`anthropic-beta` header (incl. the OAuth subscription capability), and prompt-cache
breakpoints all survive. Tested against a mock upstream (passthrough + SSE order);
clippy-clean. The full live acceptance (a real multi-turn tool-calling session) is a
manual run -- see below.

## Run it

```bash
theorem-proxy proxy            # serves on http://127.0.0.1:8788
# in another shell:
export ANTHROPIC_BASE_URL=http://127.0.0.1:8788
claude                         # this session now routes through the proxy
```

`--port` and `--upstream` are configurable. CPU-only, no model download, no config
file.

## Where it sits (the spec stack)

- `SPEC-LOCAL-PROXY-MVP.md` -- the governing spec. D1 (this passthrough) is shipped;
  D2 (native-tool membrane), D3 (ambient memory + directive injection, the value),
  D5 (brew/curl install), D6 (Commonplace sidecar) build on it.
- `SPEC-PROXY-RESIDENT-CAPABILITIES.md` -- transparent affordance execution, the
  cascade, and verification offload, once the proxy exists.
- `docs/plans/local-proxy/SPEC-PROXY-PROVE-AND-PRUNE.md` -- the five changes that make
  the harness ambient *well* and *measurable*: relevance-ranked memory injection
  (retire wholesale `MEMORY.md`, remove `recall`), tool-surface pruning, proxy-mediated
  proactive coordination, staleness-aware memory + memory CI, and built-in measurement
  with a value readout in `theorem doctor`.
- `SPEC-ONECLICK-ONBOARDING.md` -- the one-click distribution (brew, site, `theorem
  login` / `theorem doctor`) around the binary; chrome that needs this binary first.

## Notes

- Standalone Cargo root (bare `[workspace]`), like `apps/theorem-grpc`. Isolated from
  the `rustyredcore_THG` workspace; builds independently.
- Binary naming (`theorem` vs `rustyred`) is unreconciled across the specs; the crate
  is `theorem-proxy` for now and trivially renamed.
