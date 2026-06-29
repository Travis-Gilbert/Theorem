# theorem-localmodel

`theorem-localmodel` is the local face for Theorem. It wraps an
OpenAI-compatible local model, dispatches schema-guarded tool calls through MCP,
writes one JSONL ledger line per turn, and can embed `theorem-receiver` on a
dedicated OS thread.

It does not spawn Claude or Codex directly. When the model wants another head, it
uses the room tools (`coordinate` or `job_submit`), and the existing receiver
does the local launch.

## Run one turn

No config file is required for the first smoke. If `theorem-localmodel.toml` is
absent, the daemon uses safe local defaults: deterministic `rule` model, local
MCP route (`http://127.0.0.1:8380/mcp`), tenant `Travis-Gilbert`,
receiver/capture/relay off, and the ledger at `.theorem/localmodel-token-ledger.jsonl`.

```bash
cd rustyredcore_THG
cargo run -p theorem-localmodel -- --once "hello from localmodel"
```

The command prints a full JSON transcript: context reads, model decision, tool
calls, tool results, final text, and ledger status. Single-turn mode does not
start the receiver sidecar; daemon mode does.

To pin settings explicitly:

```bash
cp crates/theorem-localmodel/theorem-localmodel.example.toml ./theorem-localmodel.toml
cargo run -p theorem-localmodel -- --once "hello from configured localmodel" ./theorem-localmodel.toml
```

Switch `[model].provider` to `openai-compatible` only when a resident model is
available. Keep model keys in environment variables, not in TOML.

## Resident Gemma model host

```bash
cd rustyredcore_THG
MISTRALRS_METAL_PRECOMPILE=0 MISTRALRS_METAL_PLATFORMS=macos \
  cargo run -p theorem-localmodel -- serve ./theorem-localmodel.toml
```

`serve` uses the pinned `mistral.rs` Rust SDK directly. The default resident
model is the existing local QAT GGUF bundle at
`apps/theorem-agentd/gemma-4-12B-it-qat-UD-Q4_K_XL.gguf`, exposed as
`gemma-4-12b-it-qat` with Metal and PagedAttention requested by default. Run the
wrapper from anywhere; it executes from the repo root so this repo-relative model
path resolves without copying the 6.3 GB GGUF into the Hugging Face cache.
The upstream router provides Anthropic Messages plus model management:

- `POST /v1/messages`
- `POST /v1/messages/count_tokens`
- `GET /v1/models`
- `POST /v1/models/status`
- `POST /v1/models/unload`
- `POST /v1/models/reload`
- `GET /v1/system/info`
- `POST /v1/system/doctor`
- `GET /health`

The host uses the Hugging Face cache token by default (`token_source = "cache"`)
for tokenizer/config lookups while the large GGUF stays local. `theorem-localmodel
doctor` prints the mistral.rs doctor report plus the Theorem endpoint, model id,
quantization, and resident-memory estimate without loading the model.

Apple's build-time Metal compiler is optional for local use. `scripts/theorem`
sets `MISTRALRS_METAL_PRECOMPILE=0` and `MISTRALRS_METAL_PLATFORMS=macos` by
default, which skips build-time metallib compilation and lets `mistral.rs`
compile kernels on first model use. Install the Metal Toolchain with
`xcodebuild -downloadComponent MetalToolchain` if you prefer precompiled
metallibs.

Larger tiers for Gemma 4 26B A4B MoE and 31B dense are configured as dormant
tiers. Move a tier into `[[local_model.extra_models]]` to load it beside the
default 12B model and expose it through `/v1/models`. Optional Gemma 4 MTP
speculative decoding is configured with `[local_model.drafter]`.

## Run as a daemon

```bash
cd rustyredcore_THG
cargo run -p theorem-localmodel -- ./theorem-localmodel.toml
```

If `[receiver].enabled = true`, startup also loads `theorem-receiver.toml` and
starts `theorem_receiver::run_loop` on its own OS thread. The model loop and the
receiver communicate only through the coordination room.

`theorem-localmodel.launchd.plist.example` shows the single-job launchd shape. Do
not store real tokens in the plist; set them through launchctl or another secret
manager.

## Use through the `theorem` wrapper

The repo-level wrapper gives the install/onboarding path a single command:

```bash
scripts/theorem init
scripts/theorem once "hello from the wrapper"
scripts/theorem start
scripts/theorem serve-model
scripts/theorem model-doctor
```

`scripts/install.sh` installs that wrapper as `theorem` and can start localmodel
with the same no-config defaults.

## Tool-call guardrail

The tool catalog is the single source for:

- system prompt tool documentation
- the GBNF grammar closed over known tool names
- runtime tool-name and required-argument validation
- MCP server routing

The grammar prevents malformed JSON envelopes and hallucinated tool names. The
runtime validator rejects missing required arguments before MCP dispatch.
