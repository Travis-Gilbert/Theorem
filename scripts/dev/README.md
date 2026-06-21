# Local dev: harness MCP server + optional surfaces

The harness MCP server is `rustyred-thg-server`. It boots with **zero external
dependencies** (embedded RedCore on local disk). The gRPC, embeddings, search,
and browser surfaces are all optional and env-gated; each degrades gracefully
when its satellite isn't running.

Edit `theorem-local.env`, then:

```bash
# Terminal 1: all satellites at once (Valkey + SearXNG + embeddings + browser)
docker compose -f scripts/dev/docker-compose.yml up -d
#   first run pulls images + builds the sidecar; check health:
docker compose -f scripts/dev/docker-compose.yml ps

# Terminal 2 (optional): gRPC code search + Valkey cache
./scripts/dev/run-grpc.sh

# Terminal 3: the harness MCP server
./scripts/dev/run-harness.sh
```

Prefer not to use Docker for Valkey? `brew install valkey && valkey-server --port 6379`
works too (Valkey speaks the Redis wire protocol).

### What docker-compose brings up

| Service | Port | Used by | Notes |
|---|---|---|---|
| `valkey` | 6379 | theorem-grpc (`VALKEY_URL`) | Redis-wire cache |
| `searxng` | 8888 | harness (`SEARXNG_URL`) | JSON output enabled in `searxng/settings.yml` |
| `embeddings` | 8081 | harness (`RUSTYWEB_QWEN4B_EMBED_URL`) | TEI, bge-small (384-dim); amd64 image (emulated on Apple Silicon) |
| `browser-sidecar` | 9223 | harness (render + live action loop) | Playwright; see `browser-sidecar/` |

Swap the embedding model to the real Qwen3-Embedding-4B in `docker-compose.yml`
and set `RUSTYWEB_QWEN4B_DIMENSION=2560` in `theorem-local.env` for the
production contract. On Apple Silicon, the TEI CPU image runs under emulation;
for speed, stop that service and point `RUSTYWEB_QWEN4B_EMBED_URL` at a native
OpenAI-compatible embedder (LM Studio / llama.cpp / Ollama).

## Where each dependency lives

| Surface | Consumed by | Env var(s) | Run locally |
|---|---|---|---|
| gRPC code search | harness -> theorem-grpc | `THEOREM_GRPC_URL` (harness) | `run-grpc.sh` (binds `:50071`) |
| Valkey cache | **theorem-grpc** (not harness) | `VALKEY_URL` | a Valkey daemon |
| Embeddings | harness | `RUSTYWEB_QWEN4B_EMBED_URL` (+ `_DIMENSION`) | OpenAI-compatible embed server |
| Search | harness | `RUSTYWEB_SEARCH_PROVIDERS` + per-provider key/URL | SearXNG (or cloud keys) |
| Browser (rendered fetch) | harness | `THEOREM_SERVO_RENDER_ENDPOINT` | render sidecar |
| Browser (live action loop) | harness | `THEOREM_LIVE_BROWSER_ENDPOINT` | action sidecar |
| Visual perceiver | harness | `THEOREM_VISUAL_PERCEIVER_URL` | `uvicorn perception_visual.serve.app:app --port 8080` |

Valkey belongs to **theorem-grpc**, not the harness server: the harness server
never reads `VALKEY_URL`. So "use Valkey" = run the gRPC service against a Valkey
daemon. Valkey speaks the Redis protocol, so `redis://...` connects unchanged.

## The two browser surfaces

They are different subsystems with different jobs:

1. **Rendered-fetch escalation** (`THEOREM_SERVO_RENDER_ENDPOINT`,
   `rustyred-web` `LiveFetchOptions.rendered_endpoint`): the fetch cascade
   upgrades a JS-heavy page to a rendered DOM via a Servo/headless sidecar.
   Read-only. Unset -> the cascade does plain HTTP fetch; `web_consume` still
   works for static HTML.

2. **Live action loop** (`THEOREM_LIVE_BROWSER_ENDPOINT` +
   `THEOREM_LIVE_BROWSER_POOL_SIZE`, `rustyred-thg-server` `browser_pool.rs`
   `RemoteBrowserPool`): drives `browse_with_me` / `browse_for_me` actuation
   (click, type, navigate) against an HTTP sidecar exposing
   `sessions/checkout`, `sessions/snapshot`, `sessions/actuate`. Unset -> those
   tools fall back to the cascade (no live actuation).

3. **Visual fallback perceiver** (`THEOREM_VISUAL_PERCEIVER_URL`,
   compatibility alias `THEOREM_OMNIPARSER_URL`): when the live action sidecar
   returns a screenshot for a page with no DOM/a11y interactive elements, the
   harness calls `perception_visual` `POST /parse` and turns the parser labels
   into visual `PageState.interactive_elements`. Coordinate-synthesis clicks can
   then target those visual handles.

Both sidecars are Theorem-specific HTTP contracts (there's no off-the-shelf
binary in this repo yet), so surface 1's basic path (plain fetch) and the
read-only web tools work today; live actuation needs a sidecar that implements
that contract.

## Embeddings: what server to run

The client auto-detects the wire format from the URL:
- URL contains `/embeddings` -> **OpenAI** shape: `POST {model, input}` -> parse
  `data[].embedding`.
- otherwise -> **HuggingFace TEI** shape: `POST {model, inputs}` -> parse
  `embeddings[]`.

Any OpenAI-compatible embedding server works (vLLM, `llama.cpp --embedding`,
LM Studio, TEI w/ the openai route). The Qwen3-Embedding-4B contract is 2560-dim;
if your local model differs, set `RUSTYWEB_QWEN4B_DIMENSION` to match so vector
designations line up.
