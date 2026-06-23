# SPEC-9: CommonPlace Desktop (Tauri wrap of the Next.js workspace over the local engine)

Execution handoff. Register: enumerated deliverables, file paths, observable acceptance criteria, named choices treated as requirements, confirm-points for genuine forks. Backend infra is committable from claude.ai; the desktop shell and the CommonPlace frontend are handed to Claude Code or Codex through the harness.

## Intent

Make CommonPlace a desktop app by pointing the existing Tauri shell (`apps/desktop`) at the CommonPlace Next.js frontend and giving that frontend a transport to the local engine node the shell already hosts. This is not a new Tauri app. The shell's Rust backend already spawns and owns the local node, holds secrets in the OS keychain, runs the receiver, and drives the harness tools, which is exactly the backend a local-first CommonPlace desktop needs. The work is the frontend swap, the data transport, the native bridge, and folding the existing agent-collaborative surfaces into CommonPlace as panels.

## Grounded baseline (already built, do not rebuild)

`apps/desktop/src-tauri/src/lib.rs` is a substantial shell, not a scaffold:

- On launch, `start_local_node` spawns `rustyred_thg_server::serve_loopback(config, shutdown_rx)` bound to `127.0.0.1:17888`, with `data_dir` under AppData, `require_auth=false`, `mcp_allow_admin=true`, `mcp_default_tenant="default"`, and `allowed_origins` including `http://localhost:1420`, `http://127.0.0.1:1420`, and `tauri://localhost`. The desktop is the local node and owns its shutdown.
- `HarnessSettings { endpoint, local_endpoint, active_target, tenant, bearer_present }` switches the MCP target between the loopback node and the hosted Railway harness (`https://rustyredcore-theorem-production.up.railway.app/mcp`). `active_target` defaults to `"hosted"` today.
- Keychain via `keyring` under service `com.theorem.desktop`: `keychain_set(provider, key)` stores `provider:{provider}`; `harness_bearer_set/clear` stores the harness bearer.
- Receiver runtime: `start_receiver_locked` runs `theorem_receiver::run_loop_until` with `detect_lanes()`, worktrees, and a claim interval, spawning the local CLI heads.
- Browser tabs: `tab_create` / `tab_navigate` / `extract_visible_text` build external `WebviewWindowBuilder` windows; `agent_tab_ingest` writes captured page text into memory as `open_web_unverified`.
- Coordination: `space_bind_room`, `room_context`, `room_post_message` drive `coordination_room` / `coordination_context` / `coordinate`. Jobs: `job_submit`, `queue_status`.
- Stubs: `model_chat` returns a placeholder for every provider except Ollama; `sync_run` compiles version packs on local and hosted but reports `merged_nodes: 0`.

`apps/desktop/src-tauri/tauri.conf.json`: `build.frontendDist = "../dist"`, `build.devUrl = "http://localhost:1420"`, `beforeDevCommand = "pnpm dev"`, `beforeBuildCommand = "pnpm build"`, one window 800x600. The frontend slot is a one-line repoint.

Local engine node (`rustyred-thg-server`, `src/router.rs::build_router` plus `grpc::build_grpc_routes`) serves on loopback: `/mcp`, the items changefeed at `/v1/items/stream` and `/v1/tenants/:tenant_id/items/events` (built, this is the live channel), `/v1/coordination/events`, the full `/v1/tenants/:tenant_id/graph/*` REST surface, `/v1/tenants/:tenant_id/connectors` and `/connectors/connect`, and the browser routes `/v1/tenants/:tenant_id/browser/{web-consume,browse-with-me,browse-for-me}`. It does not mount `/graphql` on `main`.

Live GraphQL is `apps/theorem-gateway` (Axum + async-graphql, its own `Dockerfile` + `railway.toml`, `POST /graphql`, `GET /graphql` playground, `GET /scene/{id}`). It is GraphQL-over-gRPC to `theorem-grpc` (SearchService + CodeCrawlerService) plus GL-Fusion, stores nothing, and runs single-tenant (`GATEWAY_TENANT_ID`). Its surface is the public showcase set: `search`, `gapWalk`, `provenance`, `searchCode`, `exploreCode`, `codeContext`, `explainCode`, `askAgent`, `sceneForInput`, `ingestCodebase`, `reindexCodebase`.

The full workspace GraphQL schema (items, graph, memory, projection, epistemic, coordination, clusters, code) exists in `rustyredcore_THG/crates/rustyred-thg-mcp/src/graphql/` (`items.rs`, `graph.rs`, `memory.rs`, `projection.rs`, `epistemic.rs`, `coordination.rs`, `clusters.rs`, `code.rs`, `scalars.rs`, `mod.rs`). This is the surface a typed-object workspace reads and writes. It is not mounted as an HTTP endpoint on the local node as of `main`.

CommonPlace is a Next.js app living outside this repo (the forked `travisgilbert.me/commonplace`, warm amber paper, Capacities-style typed objects). `apps/copresence-editor` is the Velt + Tiptap + Yjs CRDT co-editing surface (humans co-write with Gemma), Vite + React + TS, P1 and P2 built.

## Architecture (the shape this produces)

CommonPlace (Next.js) becomes the Tauri main-window frontend. Its data layer reads and writes the local engine node over GraphQL, takes live updates from the items changefeed (`/v1/items/stream`), and reaches native capabilities (keychain, local-node control, receiver, coordination, model chat) through Tauri `invoke` against the shell's existing commands. The agent-collaborative browser, the coordination room, and the receiver fold in as CommonPlace panels rather than a separate browser-and-IDE app, because each is a coworking primitive: the browser is the human-and-agent co-browser that pulls the web into the workspace, the coordination room is the human-agent coordination surface, and the receiver is local agent execution. Default target is local (the desktop is the hub); sync to the hosted instance is the chargeable tier.

## Confirm-points (resolve before or during build)

1. GraphQL endpoint and surface coverage. The CommonPlace workspace needs items and spaces CRUD plus memory and coordination over GraphQL. As of `main`, the local node mounts no `/graphql`, and `theorem-gateway`'s schema is the showcase set without workspace CRUD. Confirm which endpoint CommonPlace's workspace queries hit: a local-node `/graphql` mounting the `rustyred-thg-mcp` schema (Deliverable 1), the hosted `theorem-gateway` extended with workspace resolvers, or a split where reads use GraphQL and writes use MCP plus the changefeed. The named default in this spec is a local-node `/graphql` over the `rustyred-thg-mcp` schema, because a local-first workspace should not depend on a hosted public gateway for its private data.

2. CommonPlace frontend location relative to the Tauri build. Tauri's `frontendDist` and `beforeBuildCommand` expect a buildable frontend the shell can reach. Confirm whether the CommonPlace Next.js app is brought into this repo (for example `apps/commonplace`) or whether the Tauri build consumes an external `travisgilbert.me` static export. The named default is to bring the CommonPlace frontend into `apps/` so one `pnpm build` produces the bundle the shell wraps.

## Deliverables

### 1. Local-node GraphQL endpoint (the workspace transport)

Mount the existing `rustyred-thg-mcp` GraphQL schema as an HTTP route on the engine so CommonPlace has one GraphQL contract locally. Add `POST /graphql` (operations) and `GET /graphql` (playground) to `rustyredcore_THG/crates/rustyred-thg-server/src/router.rs::build_router`, executing the `rustyred-thg-mcp` `QueryRoot` / `MutationRoot` against the tenant-resolved store, reusing the auth and CORS layers already applied in `build_router` and the tenant resolution in `crate::query_surface::resolve_tenant_id`. Subscriptions are empty; live updates ride `/v1/items/stream`.

Acceptance: a GraphQL query for items and a mutation creating an item both succeed against `http://127.0.0.1:17888/graphql` with `tenant` resolved, and the created item appears on `/v1/items/stream`. `GET /graphql` serves a working playground. If confirm-point 1 selects `theorem-gateway` instead, this deliverable is replaced by extending the gateway with the workspace resolvers, and the acceptance criterion moves to the gateway endpoint.

### 2. CommonPlace data layer pointed at the local node

In the CommonPlace Next.js app, configure the GraphQL client and the changefeed subscriber for desktop mode: GraphQL endpoint and changefeed URL come from runtime config (environment or a small config module), defaulting to the loopback node (`http://127.0.0.1:17888/graphql` and `http://127.0.0.1:17888/v1/items/stream`) when running inside Tauri, and to the hosted endpoints otherwise. The changefeed subscriber drives live workspace updates (new and changed items appear without a reload).

Acceptance: CommonPlace running inside the desktop reads and writes items against the local node, and a change made through one path (the UI, an agent, or a direct mutation) appears live in the UI via the changefeed.

### 3. Frontend slot repointed to CommonPlace

Build CommonPlace as a client-rendered bundle and make it the Tauri main window. Set Next.js to static export (`output: 'export'`, `images: { unoptimized: true }`) since the desktop talks to the local engine rather than a Next server. Update `apps/desktop/src-tauri/tauri.conf.json`: `frontendDist` points at the CommonPlace export directory, `devUrl` points at the CommonPlace dev server, `beforeDevCommand` and `beforeBuildCommand` invoke the CommonPlace build. The current `apps/desktop/src` Vite UI is superseded; its capabilities reappear as CommonPlace panels (Deliverable 5).

Acceptance: `pnpm tauri dev` opens a window rendering CommonPlace from the dev server, and `pnpm tauri build` produces a desktop binary whose main window is CommonPlace, with the local node live on launch.

### 4. Native bridge (typed Tauri client for the CommonPlace UI)

Expose the shell's existing `#[tauri::command]` surface to the CommonPlace UI as a typed TypeScript client, mirroring the pattern in `apps/desktop/src/lib/commands.ts`. Cover at minimum: `local_node_status`, `harness_settings_get` / `harness_settings_set`, `keychain_set` / `keychain_has` / `keychain_delete`, `harness_bearer_set` / `harness_bearer_clear`, `receiver_settings_get` / `receiver_settings_set` / `receiver_status`, `space_bind_room` / `room_context` / `room_post_message`, `job_submit` / `queue_status`, and `model_chat`. No new Rust commands are required for this deliverable; it is the TypeScript binding plus its call sites.

Acceptance: from the CommonPlace UI, a user can read local-node status, store a provider key into the keychain, toggle the receiver on and off, and post a message into a coordination room, each through `invoke` against the existing commands.

### 5. Fold the agent-collaborative browser, coordination room, and receiver in as CommonPlace panels

Re-express the shell's existing surfaces as CommonPlace panels calling the same Tauri commands and engine routes:

- Co-browser panel: the human-and-agent browser, driving the existing tab commands (`tab_create`, `tab_navigate`, `tab_reload`, `tab_set_active`, `extract_visible_text`, `agent_tab_ingest`) for the human-facing tabs, and the engine's pair co-browsing route `/v1/tenants/:tenant_id/browser/browse-with-me` (control mode `pair`) for the agent-collaborative path, so a human and an agent share one browsing surface and captured pages land in the workspace.
- Coordination panel: the room feed, participants, intents, and records via `room_context`, with posting via `room_post_message`.
- Receiver panel: receiver status, lanes, and the on/off toggle via `receiver_status` and `receiver_settings_set`.

Acceptance: from inside CommonPlace, a user can open the co-browser and ingest a page into the workspace, view the live coordination room feed, and see and toggle the receiver.

### 6. Local-first default with a sync affordance

Set `active_target` to default to `local` for the desktop so the workspace runs on the local node, with a visible action to sync to the hosted instance. The sync merge in `sync_run` is currently a pack exchange reporting `merged_nodes: 0`; the affordance is wired and surfaced now, and the real merge is tracked as a follow-up rather than a blocker for the wrap.

Acceptance: the desktop runs fully against the local node with no hosted dependency, and a sync action is present and reports its receipt honestly (including the current no-merge state) rather than claiming a merge it did not perform.

## UI fidelity gate (do not downgrade)

The CommonPlace desktop preserves the CommonPlace web design: warm amber paper, softer corners, depth and materiality, typed objects as the primary surface. The desktop wrap does not flatten CommonPlace into a generic webview chrome. Acceptance: the desktop main window is visually indistinguishable from the CommonPlace web build at the same route, and the folded panels (co-browser, coordination, receiver) adopt the CommonPlace visual language rather than the prior Vite UI styling.

## Model chat note (relation to the API-models task)

`model_chat` in the shell is the one real code gap on the desktop side and is shared with the separate API-models readiness work: routing it through the real provider invoker (the eight-provider `head_invoker`) makes provider chat usable from the CommonPlace desktop. This spec depends on that wiring for the model-driven panels but does not redefine it; it consumes `model_chat` once it calls the invoker.
