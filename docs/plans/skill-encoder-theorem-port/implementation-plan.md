# Skill Encoder -> Theorem Port (serve a self-improving Rust skill to harness agents natively)

Date: 2026-06-01
Author: Claude Code (planning; Lane G built same session, Lane S handed to Codex)
Status: PLAN + Lane G slice started. Two-repo effort (Index-API encodes;
Theorem serves). Coordinates with `Theorem/docs/plans/harness-rust-port/native-mcp-superset.md`
(same V2 server) and the Index-API skill-encoder tree
(`Index-API/docs/plans/skill-encoder/`).

Coordination note: the Python coordination MCP was down this session; durable
channel is git + path-scoped commit messages. The Theorem serving lane (Lane S)
touches the hot `rustyred-thg-mcp/src/lib.rs` Codex is actively editing
(`225f3e2` added 1,560 lines); per the project's duplicate-module lesson it is
planned + claimed for Codex, NOT speculatively built here.

## Goal (the user's words, made precise)

> "Find a bunch of Rust repos and use them to encode a new skill that lets
> agents in the harness be better at programming Rust/RustyRed. The skill itself
> should get better the more it's used, like most other skills. It's going to
> need to be ported over to Theorem to actually be used."

Two commitments fall out:

1. **Full external corpus** (Travis chose this): `rust-lang/rust` + top-20
   crates by quality + ~50 r/rust postmortems, per Skill Encoder spec
   `Index-API/Theseus/Skill encoder/Skill Encoder 1.md` §6.
2. **Served from Theorem V2 to be used.** Harness agents use the native V2 MCP
   (`rustyredcore-theorem-production`). A skill they *use* must be served from
   V2, not only produced by the Python encoder. This plan adds the serving port.

## Architecture decision: depth of the Theorem port (Travis steer 2026-06-01)

Travis: "we probably need to port that over to theorem and complete/flip it on
considering that the harness is moving there. Otherwise it would have no use."

Correct for the runtime; the open question is how deep. The encoder has two
layers that do NOT both belong in Rust:

1. **Runtime** (serve pack, `skill_apply`, run Rust validators, capture
   UseReceipt). In the agent request path -> **must be native Theorem.** If this
   is Python while the harness is on V2, the skill is stranded. Port it.
2. **Batch compile** (ingest -> tree-sitter lower -> Pairformer GNN cluster ->
   codegen -> benchmark-validate by running Claude Code). Offline job, like model
   training. Runs once per skill, not per request.

Recommended split (the pattern this codebase already uses for Modal training and
the `theseus_*` engine D1 deferral in the harness-superset plan):

- **Theorem (the harness side, fully native, zero Python):** the
  content-addressed pack store (Prolly/versioned graph, exists),
  `skill_list`/`skill_get`/`skill_apply`, native gRPC code search,
  `RecordUseReceipt` capture, and running the pack's Rust validators in-process.
  This is the agent's entire runtime path; nothing here touches Python. "Move the
  whole harness-side tool with no Python dependency."
- **Theseus (a SEPARATE offline process, stays Python, decoupled):** the entire
  encode/compile/ingest pipeline (corpus ingest -> tree-sitter lower -> GNN
  cluster -> codegen -> validate). Runs occasionally, like training. It
  PUBLISHES finished packs + lowered code atoms to Theorem by content hash, out
  of band. It is NOT a harness runtime dependency and is NOT triggered as an
  agent runtime tool (any kickoff is admin/batch, separate from the agent path).
  Ingest is "a separate process mostly," not a dependency.

Why not a full Rust rewrite of layer 2: it means porting the Pairformer GNN
(PyTorch) and the LLM-driven validation gate into a language with no ML
ecosystem - the exact category kept in Python everywhere else here (Modal
training; `theseus_*` engine). The "no Python = reliable" win is about the
request path, not an occasional batch compiler.

**RESOLVED (Travis 2026-06-01): split confirmed, framing corrected.** "We have
to move the whole harness side tool with no python dependency. I don't consider
ingesting to be a dependency, it's a separate process mostly. The compile is a
different layer that should remain Theseus and we can port the skills over." So:
the harness USE side moves to Theorem fully native, zero Python in the agent
path. The compile/ingest stays Theseus as a SEPARATE offline process - NOT a
runtime dependency reached via live dispatch. The seam is a content-addressed
PUBLISH (encoder -> Theorem store, out of band), not a harness->Python call. The
encoder is not rewritten; the compiled skills are ported (served) in Theorem.
"Connection points" = the harness is in Theorem, so usage must connect there;
it does not mean the harness calls Python at runtime.

## What already exists (grounded 2026-06-01)

The encoder is built in **Index-API (Python)**, not Theorem:

- `apps/notebook/encode/` (17 modules). Public API
  (`apps/notebook/encode/__init__.py`): `lower_code_corpus_source_packet`,
  `build_capability_pack_spec`, `canonical_content_hash`,
  `run_encoding_pipeline`, `CODE_CORPUS_SOURCE_CLASS='code_corpus_v1'`,
  `UseReceipt`, `CapabilityOperator`, `EncodingSchema`.
- `apps/notebook/encode/codegen/rust.py` (native Rust validator codegen).
- `apps/notebook/cold_tier/client.py` (S3 content-addressed cold tier).
- `apps/notebook/domain_packs/manifests/rust.yaml` (the Rust pack, `is_active: false`).
- `apps/notebook/pairformer/cluster_abstraction.py` (cluster scaffold).
- `apps/notebook/encode/benchmarks.py` (validation gate runner).
- Registry: `apps/orchestrate/registry/capability_packs.py` (`CapabilityPackSpec`,
  `kind='skill_pack'`). Promotion states in `encode/contracts.py::PROMOTION_STATES`
  (`draft -> shadow -> advisory -> validated -> canonical -> retired`).
- MCP verbs `code_encode` / `code_compile_pack` / `code_validate_pack`
  (`mcp_server_theorem/tools/encode.py`).

Theorem provides the **substrate** the serving port needs (already built):

- Content-addressed Prolly/versioned graph:
  `rustyredcore_THG/crates/rustyred-thg-core/src/versioned_graph.rs`; the
  `rustyred_thg_graph_version_*` verbs are LIVE on V2 (compile/diff/merge/log).
- The RustyWeb crawler (`rustyredcore_THG/crates/rustyred-web`) for fetching
  repos off the web.
- The native MCP (`rustyred-thg-mcp`) where V2's tools live.

What does NOT exist yet (the build):

1. A `code_repo` ingest worker. `rust.yaml` references `worker:
   local_filesystem`, which is **not in `WORKER_REGISTRY`**
   (`apps/notebook/domain_packs/workers.py` has only openalex/arxiv/
   semanticscholar/static_corpus). No git/repo/code worker -> the full corpus
   cannot ingest. **Lane G.**
2. The held-out validation gate (only a quick-look verb shipped). **Lane E.**
3. The Theorem serving surface: a `skill_pack` store in RustyRed + native MCP
   `skill_*` verbs so agents pull/apply packs on V2. **Lane S.**

## The seam (why the port is small)

Both sides speak content-addressing, so the pack crosses by hash, not by
rewrite:

```
Index-API (Python encoder)                    Theorem (Rust, V2)
--------------------------                     ------------------
code_repo ingest (Lane G)
  -> source_packet (root+paths, rust)
  -> lower_code_corpus_source_packet
  -> build_capability_pack_spec
       kind='skill_pack'
       metadata.pack_content_hash  ───────────►  skill_pack node in RustyRed
       artifacts (Rust validators,                 (content-addressed, Prolly)
        decision trees, templates,         ◄───── native MCP skill_* verbs
        embeddings)                                 (skill_list/get/apply)
                                                  harness agents pull via V2
UseReceipt / fitness / promotion  ◄───────────  use + outcome signal
  (self-improvement loop, Lane F)                 mirrored back as signal
```

The pack is the content-addressed artifact. The cold tier holds the source
bytes; the pack metadata holds `pack_content_hash` + `source_content_hash` +
artifact hashes. Theorem stores the pack node by that hash and serves it. The
encoder never moves to Rust; only the serving does.

## Lanes

### Lane G - full Rust corpus + ingest worker (Index-API, MINE, started this session)

- [x] **G0** (DONE 2026-06-01, Index-API `043bf1c6`) Author the full-corpus definition (the "bunch of Rust repos"):
  `rust-lang/rust` + top-20 crates by quality + RustyRed in-repo crates +
  postmortem source strategy. Built this session into
  `apps/notebook/domain_packs/manifests/rust.yaml` (corpus section), kept
  `is_active: false` (gated on G1 + Lane E).
- [x] **G1** (DONE 2026-06-01, Index-API `996c7cb2`; 5 tests green, verified
  807 atoms from 15 files of rustyred-thg-core) Build the `code_repo` ingest
  worker in `apps/notebook/domain_packs/workers.py` + register in `WORKER_REGISTRY`.
  Contract: for each repo (git clone or local root) -> walk `*.rs` (bounded by
  `max_files`) -> build a `code_corpus_v1` `source_packet` (`root`+`paths`,
  `language='rust'`) -> `lower_code_corpus_source_packet(plan, packet)` ->
  `build_capability_pack_spec(...)` -> write source bytes to cold tier
  (`cold_tier/client.py`) -> tag objects with pack provenance (reuse
  `_tag_objects_for_pack`). Replace `rust.yaml`'s dangling `local_filesystem`
  reference with `code_repo`.
- [ ] **G2** Postmortem ingest: curate ~50 r/rust + named-blog postmortems into
  a `static_corpus` JSONL (`domain_packs/static_corpora/rust_postmortems.jsonl`)
  via the existing `static_corpus` worker; or a `web_crawl` worker over the
  RustyWeb crawler. (Curation is a real task; do not fabricate URLs.)
- [ ] **G3** Run the ingest (gated on infra: S3 cold-tier creds + disk/compute
  to clone `rust-lang/rust` + 20 crates). Produces source packets in cold tier.

### Lane E - encode + validate + promote (Index-API, mostly exists)

- [ ] **E1** Run `run_encoding_pipeline` over the lowered Rust views ->
  candidate patterns (`pairformer/cluster_abstraction.py`) -> compile via
  `codegen/rust.py` -> `build_capability_pack_spec` -> `skill_pack` candidate.
- [x] **E2** Held-out validation task set (shipped 2026-06-05, Index-API):
  `apps/notebook/encode/validation_tasks/rust_refactoring_v1.jsonl` now carries
  the full 20 real Rust refactoring tasks against Theorem/RustyRed files, with
  acceptance criteria and validator ids. This resolves the old 3-5 seed-only
  working assumption; the full 20-task floor is now data.
- [x] **E3** Full held-out runner in `encode/benchmarks.py` (shipped
  2026-06-05, Index-API): `load_rust_held_out_task_set` +
  `rust_held_out_validation_gate` score baseline vs treatment `UseReceipt`s,
  enforce the 20-task floor, require treatment coverage, require
  `validator_pass_rate >= 0.95`, and emit
  `benchmark_treatment_beats_baseline` / `canonical_ready`. The runner is
  wired; actual pack promotion still waits on real baseline/treatment receipts
  from a live run.
- [ ] **E4** Flip `rust.yaml` `is_active: true` once E3 passes.

### Lane S - Theorem serving surface (Rust, V2, CODEX - hot lib.rs, claim first)

- [x] **S1** `skill_pack` node contract in the runtime (shipped 2026-06-05):
  `theorem-harness-runtime::skill_pack` stores `CapabilityPackSpec` JSON as
  content-addressed `SkillPack` graph nodes keyed by `pack_content_hash`, with
  `SKILL_PACK_SOURCE` and `SKILL_PACK_ARTIFACT` edges to source/artifact hash
  nodes. It also persists `SkillPackUseReceipt` nodes and
  `SKILL_PACK_APPLIED` edges for the use loop.
- [x] **S2** Native MCP verbs on V2 (Form-B, shipped 2026-06-05):
  `skill_list` and `skill_get` are advertised in read-only mode;
  `skill_publish` and `skill_apply` are write-mode tools. The MCP round trip is
  covered by `native_skill_pack_tools_round_trip_through_mcp`. The source
  Theorems Harness plugin route policy now advertises the same `skill_*` verbs
  and forwards them through the native MCP route with route receipts.
- [x] **S3a** Safe native validator declarations (shipped 2026-06-05):
  `skill_apply` runs deterministic validator declarations in-process
  (`required_field`, `context_required_field`, `artifact_hash_present`,
  `always_pass`) and persists the receipt with `validator_execution_mode:
  safe_declaration`.
- [x] **S3b** Bounded native validator artifact execution (shipped 2026-06-05):
  `skill_apply` now loads `native_validator_candidate` / Rust validator artifact
  descriptors from the published pack metadata and runs the code-member
  signature predicate in a bounded native sandbox with
  `validator_execution_mode: native_artifact_sandbox`. It does not compile or
  execute arbitrary uploaded Rust source in the MCP request path.
- [ ] **S3c** Optional compiled-artifact runner: if the encoder later publishes
  packaged validator crates or WASM artifacts, add a separate bounded runner
  with process/wasm isolation and explicit operator policy. Do not fold raw
  source execution into `skill_apply`.
- [x] **S4a** Native gRPC code search first lane = the agent query surface for "be better at
  Rust" (Travis: "the gRPC external codebase search would be useful for this").
  Theorem now has a native first lane in `apps/theorem-grpc`:
  `theorem_code.v1.CodeCrawlerService` exposes `IngestCodebase`,
  `ReindexCodebase`, `SearchCode`, `RecognizeCode`, `ExploreCode`,
  `CodeContext`, `ExplainCode`, and `RecordUseReceipt`; it stores
  repo/file/symbol nodes, Rust AST-backed call/dependency graph edges, trust
  tiers, community ids, and operation/use receipts in RedCore and is also
  reachable through `theorem_grpc.code_search.*` app affordances.
- [ ] **S4b** Bring the native lane to full CodeCrawler parity:
  - Contract: `Index-API/protos/theorem/v1/code_crawler.proto`
    (`CodeCrawlerService`): `Search`, `Recognize`, `Explore`, `Context`,
    `Explain`, `RecordUseReceipt`. `CodeSymbol` carries `trust_tier`
    (advisory/validated/canonical, the SAME ladder as skill packs) and
    `community_id`.
  - Python impl exists: `Index-API/apps/theorem_grpc/code_crawler.py`
    (Search/Context/Explore/Explain/RecordUseReceipt live; `Ingest`/`Impact`
    NOT implemented - ingest stays REST, see Lane G).
  - Native precedent exists: `Theorem/apps/theorem-grpc` already serves
    `theseus_search.v1.SearchService` in pure Rust over the RustyRed substrate
    (the URL-swap connection-point pattern via `THEOREM_SEARCH_URL`).
  - **Shipped native subset:** `Recognize`, `Explore`, `Explain`,
    `RecordUseReceipt`, advisory trust tiers, community ids, and Rust
    AST-backed call/dependency graph edges now live on top of the native RedCore
    code graph and the `theorem_grpc.code_search.*` affordance path. Harness
    agents can query symbols, context, graph neighborhoods, explanations, and
    use receipts without calling the Python CodeCrawler.
  - **Remaining parity:** wire the deployed MCP product backend to the live
    theorem-grpc app-affordance transport and add parser-grade dependency/call
    graph extraction for non-Rust languages where the Python CodeCrawler
    contract requires it.
  - `RecordUseReceipt` over this gRPC IS the self-improvement connection point
    (Lane F): every agent code query/use emits a receipt that drives
    fitness/promotion. The seam already exists in the proto.
  - Precondition: the code atoms (`CodeFile`/`CodeMember`/`CodeSymbol`) must
    live in the RustyRed substrate for the native gRPC to search them. Today
    they are in Memgraph (Python ingest). Landing them in RustyRed is shared
    work with Lane G's `code_repo` ingest + the content-addressed `skill_pack`.

Connection-point framing (corrected per Travis): "connection points" means the
harness is IN Theorem, so to be USED a skill must be served there - the whole
harness-side toolset is native with zero Python dependency. It does NOT mean the
harness dispatches to a Python encoder at runtime. S2 (`skill_*` verbs) + S4
(native gRPC code search) are the native use surface; the ONLY cross-boundary
link is `skill_publish` - the offline Theseus encoder pushing a finished,
content-addressed pack into Theorem, out of band. Ingest is a separate process,
not a harness runtime dependency. That decoupling is what makes the
full-harness-on-Theorem move "work better for everything."

### Lane H - harness agents actually use it (ties to harness-superset Lane T)

- [ ] **H1** The lift/push path: when a harness agent's trajectory matches the
  Rust pack's trigger, V2 surfaces `skill_apply`. This rides the same native
  surface the harness-superset plan makes writable (Lane O deploy + write-mode).
- [ ] **H2** Acceptance = a harness agent doing a real Rust task on V2 pulls the
  pack via `skill_get`/`skill_apply` and its Rust validators fire. (Real
  surface, not a demo route.)

### Lane F - self-improvement loop (exists Python; mirror to Theorem)

- [ ] **F1** Each `skill_apply` emits a `UseReceipt` (`encode/contracts.py`) with
  the outcome.
- [ ] **F2** `encode/fitness.py` scores it; `schema_memory.py` updates priors;
  `promotion_router.py` + `rust.yaml` policy act (`on_negative_signal:
  weaken_schema_prior`, `on_validator_failure: retire_operator`).
- [ ] **F3** Pairformer (`learned_scorer.py`) trains on the UseReceipts to learn
  which pack to push for which trajectory. This is the "gets better the more
  it's used" loop; it is the same substrate as the harness `encode` memory verb.

## Build sequence

1. Lane G (corpus + worker) - mine, started this session. G1 worker is the
   unblock; G3 run needs infra.
2. Lane E (encode + validate) - E2/E3 are shipped as a full 20-task task set
   plus pure receipt-scoring gate. E1 still needs a real encoded Rust pack run;
   E4 remains gated on live baseline/treatment receipts passing E3.
3. Lane S (Theorem serving) - Codex, after claiming the hot lib.rs seam.
4. Lanes H + F - integration, after S + E.

## What needs whom / what

- **Mine, buildable now (no infra):** G0 (corpus def), G1 (code_repo worker),
  E2/E3 (validation task corpus + pure runner), the plan.
- **Needs your infra/greenlight:** G3 (S3 cold-tier creds + clone compute),
  E3 run (Claude Code on the task set), Lane O deploy + write-mode on V2.
- **Codex (Theorem MCP territory):** Lane S (claim the lib.rs seam first).

## Acceptance (the floor)

- The full Rust corpus is defined (G0) and ingestible by a real worker (G1),
  not a dangling `local_filesystem` reference.
- A `skill_pack` compiles from the corpus with content-addressed provenance
  (`pack_content_hash`) and passes the held-out gate (E3).
- V2 serves the pack natively (`skill_list`/`skill_get`/`skill_apply`), with
  safe validator declaration receipts and bounded native artifact-descriptor
  execution shipped.
- A harness agent on V2 pulls + applies the pack on a real Rust task (H2).
- Use emits UseReceipts that drive promotion/retirement (F1-F3): the skill
  measurably self-improves.

## Codex handoff (Lane S)

Codex claimed and shipped the first Lane S slice on 2026-06-05. The seam
contract remains the `CapabilityPackSpec` JSON `build_capability_pack_spec`
emits (`apps/notebook/encode/code_corpus.py:118`): `id`, `kind='skill_pack'`,
`capabilities`, `validators`, `metadata.pack_content_hash`, and
`metadata.artifacts`.

The next Codex/Claude work should start from the shipped Rust surface:
`theorem-harness-runtime::skill_pack` plus MCP verbs `skill_list`,
`skill_get`, `skill_publish`, and `skill_apply`. Do not create a duplicate
registry. The source plugin already routes those verbs through native MCP. The
Lane E task corpus and pure receipt-scoring gate are now shipped in Index-API;
the next implementation gap is running E1 and feeding real baseline/treatment
receipts through E3 before E4 flips `rust.yaml` active. Compiled crate/WASM
validator execution remains a separate optional S3c runner, not an MCP
request-path source execution feature.
