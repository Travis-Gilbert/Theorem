# theorem-gateway

Theorem's browser-facing GraphQL gateway. An Axum + async-graphql server whose
resolvers call the gRPC services that already exist (`theorem-grpc`:
`theseus_search.v1.SearchService` + `theorem_code.v1.CodeCrawlerService`) plus
the GL-Fusion model endpoint, and shape the results for the web. It stores
nothing durable: a front door and a translator. It mirrors
`our-civic-atlas-backend`'s GraphQL-over-gRPC shape (Axum + async-graphql +
tonic), pointed at the showcase capabilities instead of civic data.

Standalone Cargo root (NOT a member of `rustyredcore_THG`); own `Dockerfile` +
`railway.toml` (its own Railway service). Unlike `theorem-grpc` it has **no
path-deps into the THG crates**: it only needs external crates plus the vendored
protos.

## Build & run

```bash
cd apps/theorem-gateway
cargo build -p theorem-gateway          # build
cargo test  -p theorem-gateway          # unit tests (pure logic + schema shape)
cargo run   -p theorem-gateway -- --export-schema > schema.graphql  # emit SDL
PORT=50080 cargo run -p theorem-gateway # serve locally
```

- `POST /graphql` GraphQL operations
- `GET  /graphql` GraphiQL playground
- `GET  /scene/{id}` SceneOS scene page (self-contained HTML; SceneOS add-on)
- `GET  /healthz` liveness (`ok`)

Binds `0.0.0.0:$PORT` (default `50080` local). Reaches `theorem-grpc` over
`THEOREM_GRPC_URL`.

## Environment

| Var | Meaning | Default |
|-----|---------|---------|
| `PORT` | HTTP listen port | `50080` |
| `THEOREM_GRPC_URL` | theorem-grpc address (bare `host:port` is normalized to `http://`) | `http://theorem-grpc.railway.internal:8080` |
| `GATEWAY_TENANT_ID` | tenant for every code-graph op (ingest + search share it) | `gateway-public` |
| `GLFUSION_URL` | GL-Fusion model endpoint; unset => askAgent returns context + an honest "not configured" answer | (unset) |
| `GLFUSION_TOKEN` | optional bearer for GL-Fusion; **server-side only**, never returned | (unset) |
| `GLFUSION_MODEL` | model name reported when GL-Fusion does not echo one | `theseus-gemma-31b-glfusion` |
| `CORS_ALLOW_ORIGINS` | comma-separated web origins; empty => permissive (dev) + warning | (unset) |
| `VALKEY_URL` | Valkey/Redis for rate-limit counters + response cache; unset => in-memory limiter, no cache | (unset) |
| `VALKEY_CACHE_TTL_SECONDS` | TTL for cached read responses | `60` |
| `PUBLIC_INGEST_ALLOWLIST` | comma-separated repo-URL prefixes `ingestCodebase` accepts; empty => fail closed | (unset) |
| `GATEWAY_RATE_LIMIT_BURST` | token-bucket burst per IP | `10` |
| `GATEWAY_RATE_LIMIT_PER_MINUTE` | token-bucket refill per IP | `20` |
| `GATEWAY_INGEST_WAIT_SECONDS` | how long `ingestCodebase` polls the async job before returning the ack | `180` |
| `GATEWAY_PUBLIC_URL` | base URL for absolute `SceneRef.url`; unset => relative `/scene/{id}` | (unset) |
| `GATEWAY_SCENE_CACHE_SIZE` | max compiled scenes held in the in-memory store | `256` |

`GLFUSION_TOKEN` and the internal gRPC/Valkey addresses stay server-side and
appear in no browser-visible response.

## GraphQL surface

Queries (read-only, directly demoable):
- `search(query, mode): [SearchHit!]!` -> SearchService.Search
- `gapWalk(seed): KnowledgeGraph!` -> SearchService.GapWalk (single-round PPR)
- `provenance(nodeId): GraphNode` -> SearchService.Provenance
- `searchCode(query, repoId): [CodeSymbol!]!` -> CodeCrawlerService.SearchCode
- `exploreCode(symbolId): KnowledgeGraph!` -> CodeCrawlerService.ExploreCode
- `codeContext(repoId, target): CodeContextBlock!` -> CodeCrawlerService.CodeContext
- `explainCode(symbolId): CodeSymbol!` -> CodeCrawlerService.ExplainCode
- `askAgent(question, scope): AgentAnswer!` -> graph-context assembly + GL-Fusion
- `sceneForInput(input, scope, origin): SceneRef!` -> SceneOS scene compile (add-on)

Mutations (rate-limited):
- `ingestCodebase(repoUrl): IngestReceipt!` -> CodeCrawlerService.IngestCodebase
- `reindexCodebase(repoId): IngestReceipt!` -> CodeCrawlerService.ReindexCodebase

`askAgent` is the showpiece: it assembles a graph context (code-KG via
searchCode -> exploreCode -> codeContext for `{ repoId }` scope, or instant-KG
via gapWalk for `{ seed }` scope), sends it to GL-Fusion, and returns
`AgentAnswer { answer, contextNodes, sources, model }`. `contextNodes` is the
differentiator: the UI renders the answer next to the graph the model read.

## SceneOS add-on (instant-KG visualization)

Turns the instant-KG / code-KG result into a SceneOS scene: nodes spawn from the
omni-bar origin, settle into a community-colored force layout, and the GL-Fusion
explanation reveals as d3 callouts pinned to the graph.

- **Deliverable A** (`src/schema/scene.rs`): `sceneForInput(input, scope, origin)`
  reuses the askAgent context assembly, computes degree centrality + WCC
  communities + backbone edges, anchors the model's explanation as an annotation
  atom, compiles a `force_graph` `ScenePackageV2` via `scene-os-core`, stores it
  in a bounded content-hash-keyed in-memory store, and returns
  `SceneRef { sceneId, url }`.
- **Deliverable B** (`src/scene_serve.rs`): `GET /scene/{id}` renders the package
  as one self-contained HTML asset via `scene-os-web::render_scene` (the
  `scene_payload_json` escape path: `<`, `>`, `&` -> `\uXXXX`). Unknown id => the
  honest empty state with a 404.
- **Deliverable C** (`scene-os-web/web/src` + rebuilt `dist`): the d3 renderer
  gained an origin-spawn -> force-settle -> annotate enter choreography (rAF tween
  with eased fade-in), a `prefers-reduced-motion` path that reveals the settled,
  annotated layout directly, and scoped annotation callouts (annotation atoms,
  `kind:"annotation"`, are excluded from the force layout and pinned to their
  anchor node). Rebuild with `cd rustyredcore_THG/crates/scene-os-web/web &&
  npm run build` (the gateway embeds `dist/scene-os.bundle.js` via `include_str!`).
- **Deliverable D** (the Next.js `/experiments` omni-bar tile) is a separate-repo
  consumer: this repo has no Next.js frontend (`apps/desktop` is Tauri). The tile
  POSTs `sceneForInput` with the bar's screen coords as `origin` and embeds
  `GET /scene/{id}`; the gateway provides exactly that surface.

The gateway path-deps into `scene-os-core` + `scene-os-web` for this (the one
place the core gateway's "no THG path-dep" property is reversed); the Dockerfile
copies the whole `rustyredcore_THG/crates` dir to stay build-stable.

## Security model

- `/graphql` is the only browser boundary. CORS restricts which origins may
  call it.
- `ingestCodebase` accepts only repo URLs matching `PUBLIC_INGEST_ALLOWLIST`; a
  non-allowlisted URL is refused **without** any gRPC call (an empty allowlist
  fails closed).
- `ingestCodebase`, `reindexCodebase`, and `askAgent` are rate-limited per IP
  (token bucket: in-memory by default, atomic Valkey Lua script when
  `VALKEY_URL` is set). Read queries are open.

## Deploy

Its own Railway service. `railway.toml` + `Dockerfile`, build context = Theorem
repo root. Set `THEOREM_GRPC_URL=http://theorem-grpc.railway.internal:8080`, a
public domain, and `CORS_ALLOW_ORIGINS` to the website origin. Health at
`/healthz`. Commit the exported `schema.graphql` for frontend codegen.

## One open item (confirm before relying on `askAgent` answers)

The exact GL-Fusion serving contract — the HTTP request/response schema of
`ghcr.io/travis-gilbert/theseus-gemma-31b-glfusion` as deployed, specifically
how it accepts graph context as a first-class structured input. It is isolated
to `GlFusionClient::ask` in `src/clients.rs`:

- **Request (provisional):** POSTs both a structured `graph_context`
  (`{ nodes, edges, sources }`) **and** a flattened `prompt` (the same context
  as text), so either a structured-input or prompt-only serving path works.
- **Response (defensive):** parses the answer from `answer` / `text` / `output`
  / `response` / `generated_text`, then OpenAI-style
  `choices[0].message.content` / `choices[0].text`; model name from `model` else
  `GLFUSION_MODEL`.

Confirming the real contract is a one-method edit; no resolver changes needed.
Everything else (search, code KG, ingest, explore, context, explain) is
grounded against theorem-grpc today and works without it.

## Spec divergences (code-grounded, surfaced not buried)

1. **Client generated from vendored protos, not the `theseus-client` crate.**
   The spec suggests reusing `theseus-client` for SearchService, but that crate
   lives in `our-civic-atlas-backend` and is not reachable as a path-dep from
   Theorem. Deliverable 1's explicit `build_client(true)` over the same vendored
   protos generates a wire-identical client; that is what ships here.
2. **`ingestCodebase` is async + polled.** The code proto is job-based
   (`IngestCodebase` returns `job_id` + `status: "submitted"`; the worker
   clones/parses/commits). To satisfy AC2 (a following `searchCode` sees the
   graph), the resolver submits then polls `GetIngestStatus` to a terminal state
   within `GATEWAY_INGEST_WAIT_SECONDS`, returning real counts. On timeout it
   returns the ack marked `running` with `jobId` to poll. `IngestReceipt` gains
   `jobId`, `status`, `message` fields beyond the spec's four (additive).
3. **`IngestReceipt.edgeCount` is honestly `0`.** The ingest response surfaces
   `symbols_indexed` (mapped to `nodeCount`) but no edge count; per-symbol edges
   are exposed via `exploreCode`.
4. **Tenant is gateway-config, not per-request.** All code ops use
   `GATEWAY_TENANT_ID` so ingest and search land on the same tenant (the public
   demo is single-tenant). `searchCode(query, repoId)` carries no tenant arg.
5. **`AppAffordanceService` is not compiled.** Not needed for v1 of the gateway
   (per the spec); only the two resolved services are built into the client.
