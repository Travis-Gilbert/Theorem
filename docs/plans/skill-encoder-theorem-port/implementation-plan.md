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

- **Native on Theorem:** the content-addressed pack store (Prolly/versioned
  graph, exists), `skill_list`/`skill_get`/`skill_apply`, UseReceipt capture, and
  tree-sitter **Rust ingestion** (the Skill Encoder spec §4 explicitly wants this
  RustyRed-native; tree-sitter has Rust bindings). No Python in any agent path.
- **Python, dispatched by a native `skill_encode` verb (Modal pattern):** the
  heavy GNN clustering + codegen + benchmark validation. The harness fires a
  native verb; a Python batch worker compiles; the pack lands back in Theorem by
  `pack_content_hash`.

Why not a full Rust rewrite of layer 2: it means porting the Pairformer GNN
(PyTorch) and the LLM-driven validation gate into a language with no ML
ecosystem - the exact category kept in Python everywhere else here (Modal
training; `theseus_*` engine). The "no Python = reliable" win is about the
request path, not an occasional batch compiler.

**RESOLVED (Travis 2026-06-01): split confirmed.** "The harness is going to be
moved fully to Theorem soon so it's just a matter of the connection points. I
think it should work better for everything." So the encoder is NOT rewritten;
the runtime + ingestion go native, the heavy compile stays Python and is reached
through a connection point (a native dispatch verb). The work is the connection
points, not a port of `encode/`.

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

- [ ] **G0** Author the full-corpus definition (the "bunch of Rust repos"):
  `rust-lang/rust` + top-20 crates by quality + RustyRed in-repo crates +
  postmortem source strategy. Built this session into
  `apps/notebook/domain_packs/manifests/rust.yaml` (corpus section), kept
  `is_active: false` (gated on G1 + Lane E).
- [ ] **G1** Build the `code_repo` ingest worker in
  `apps/notebook/domain_packs/workers.py` + register in `WORKER_REGISTRY`.
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
- [ ] **E2** Author the held-out validation task set (spec §6: 20 real Rust
  refactoring tasks; `open-questions.md` Q2). Seed 3-5 against `rustyredcore_THG`
  first; full 20 is the bar for canonical.
- [ ] **E3** Wire the full held-out runner in `encode/benchmarks.py` (baseline
  vs treatment). Promote per `rust.yaml` policy: `validator_pass_rate >= 0.95`
  AND `benchmark_treatment_beats_baseline`.
- [ ] **E4** Flip `rust.yaml` `is_active: true` once E3 passes.

### Lane S - Theorem serving surface (Rust, V2, CODEX - hot lib.rs, claim first)

- [ ] **S1** `skill_pack` node contract in the runtime (new module, mirror
  `coordination.rs` / the planned `memory.rs`): store a `CapabilityPackSpec`
  by `pack_content_hash` as a content-addressed node in RustyRed, with edges to
  source/artifact hashes. Reuse the Prolly/versioned graph
  (`versioned_graph.rs`).
- [ ] **S2** Native MCP verbs on V2 (Form-B): `skill_list` (available packs),
  `skill_get` (pack metadata + artifacts by id/hash), `skill_apply` (load a
  pack's artifacts into the agent's context / run the Rust validators),
  `skill_publish` (accept a pack from the Python encoder by hash). Read verbs in
  read-only mode; `skill_publish`/`skill_apply` behind write mode (Lane O of the
  superset plan).
- [ ] **S3** Run the pack's native Rust validators in-process (the payoff of a
  Rust corpus: the validators ARE Rust, so they execute natively in Theorem,
  not as advisory strings).
- [ ] **S4** Native gRPC code search = the agent query surface for "be better at
  Rust" (Travis: "the gRPC external codebase search would be useful for this").
  There is already a contract and both a Python impl and a native precedent:
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
  - **Build:** implement `code_crawler.proto` natively in Theorem as a sibling
    of `theseus_search` (search the code atoms in RustyRed). This is how harness
    agents query the encoded Rust corpus - symbols, context, call graphs,
    explanations - not just run validators. It is the runtime payoff of the
    whole encode.
  - `RecordUseReceipt` over this gRPC IS the self-improvement connection point
    (Lane F): every agent code query/use emits a receipt that drives
    fitness/promotion. The seam already exists in the proto.
  - Precondition: the code atoms (`CodeFile`/`CodeMember`/`CodeSymbol`) must
    live in the RustyRed substrate for the native gRPC to search them. Today
    they are in Memgraph (Python ingest). Landing them in RustyRed is shared
    work with Lane G's `code_repo` ingest + the content-addressed `skill_pack`.

Connection-point framing (Travis: "just a matter of the connection points"):
S2 (`skill_*` MCP verbs) serve the compiled artifacts; S4 (native code-search
gRPC) serves queries over the ingested Rust corpus graph; both point at the same
RustyRed substrate. Neither requires porting `encode/`. The Python encoder
publishes packs + code atoms by content hash; Theorem serves them. That is the
"it should work better for everything" the full-harness-on-Theorem move buys.

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
2. Lane E (encode + validate) - mostly exists; E2/E3 (validation tasks + gate)
   are the real remaining build.
3. Lane S (Theorem serving) - Codex, after claiming the hot lib.rs seam.
4. Lanes H + F - integration, after S + E.

## What needs whom / what

- **Mine, buildable now (no infra):** G0 (corpus def), G1 (code_repo worker),
  E2 (validation task seed), the plan.
- **Needs your infra/greenlight:** G3 (S3 cold-tier creds + clone compute),
  E3 run (Claude Code on the task set), Lane O deploy + write-mode on V2.
- **Codex (Theorem MCP territory):** Lane S (claim the lib.rs seam first).

## Acceptance (the floor)

- The full Rust corpus is defined (G0) and ingestible by a real worker (G1),
  not a dangling `local_filesystem` reference.
- A `skill_pack` compiles from the corpus with content-addressed provenance
  (`pack_content_hash`) and passes the held-out gate (E3).
- V2 serves the pack natively (`skill_list`/`skill_get`/`skill_apply`), Rust
  validators run in-process (S3).
- A harness agent on V2 pulls + applies the pack on a real Rust task (H2).
- Use emits UseReceipts that drive promotion/retirement (F1-F3): the skill
  measurably self-improves.

## Codex handoff (Lane S)

Claim the `rustyred-thg-mcp/src/lib.rs` + a new runtime `skill_pack` module
before building (hot file; duplicate-module lesson). The seam contract is the
`CapabilityPackSpec` JSON `build_capability_pack_spec` emits
(`apps/notebook/encode/code_corpus.py:118`): `id`, `kind='skill_pack'`,
`capabilities`, `validators`, `metadata.pack_content_hash`,
`metadata.artifacts`. Store by `pack_content_hash` in the versioned graph;
expose `skill_*` verbs; run the Rust artifacts natively. Coordinate with the
harness-superset plan (same V2 server, same write-mode gate).
