# Lane A: Grounded Skill API / SPA (spec 1)

Date: 2026-06-02
Owner: Claude Code
Source: `~/Downloads/grounded-skill-api-spec.md`
Builds on (extends, does not duplicate):
`Theorem/docs/plans/skill-encoder-theorem-port/implementation-plan.md`,
`Index-API/docs/plans/skill-encoder/`.

## Product (spec one-liner)

Query or upload code -> a complete, executable, epistemically grounded Agent
Skill in the open Agent Skills standard, runnable in any compliant agent (Claude
Code, Codex CLI, Cursor, Gemini CLI, Copilot) with no Theorem harness at runtime.
The grounding is the moat (correct, provenance-backed code); the open standard is
the distribution.

## What exists vs what is new

Exists (Python encoder, do not rebuild): `build_capability_pack_spec` ->
`CapabilityPackSpec` (a `skill_pack` node: id, capabilities, validators,
metadata.pack_content_hash, artifacts), `run_encoding_pipeline`, `UseReceipt`,
content-addressing, the Rust corpus worker.

New (this spec, the product layer on top):
1. Open Agent Skills standard EXPORTER. CapabilityPackSpec -> a skill FOLDER. No
   exporter exists today (grep empty 2026-06-02).
2. Provenance block as the product trust spine.
3. Upload input -> per-tenant scope.
4. Qwen3-Embedding-8B + Qwen3-Reranker-8B upgrade + vector backfill into HNSW.
5. Dual surface: harness plugin (internal) + API/SPA (external).

## Checklist (every item backrefs a spec section)

### A1 Open-standard exporter (the output contract) - DONE 2026-06-02 (Index-API `apps/notebook/encode/skill_export.py` + tests, 6 green)
- [x] A1.1 `skill_export` module: CapabilityPackSpec -> Agent Skills folder.
  SKILL.md frontmatter `name` + `description` written AS the verbs a user types,
  for trigger reliability. [spec "The output contract" para 1]
- [x] A1.2 scripts/ folder: emit the pack's validators/templates as executable,
  runnable code (Python / TypeScript / shell as the task dictates). [spec "A
  scripts folder with the executable code that is the actual capability, tested
  and runnable"]
- [x] A1.3 Optional reference files for detailed signatures / schemas. [spec
  "Optional reference files for detailed signatures or schemas"]
- [x] A1.4 Provenance block (frontmatter or sidecar): which corpus, at what
  confidence, with links to sources. [spec "a provenance block ... recording
  which corpus the skill was distilled from, at what confidence, with links"]
- [x] A1.5 Test-first: the emitted folder validates against the open Agent Skills
  standard (frontmatter parses, scripts run, provenance present). [spec "tested
  and runnable", "the slice that proves the output contract ... end to end"]

### A2 Embedder upgrade + backfill (shared seam 1)
- [ ] A2.1 Qwen3-Embedding-8B Modal app (code + text nodes), instruction-aware.
  [spec "Embedding models ... Qwen3-Embedding ... instruction-aware"]
- [ ] A2.2 Qwen3-Reranker-8B Modal app (cross-encoder); retrieve-then-rerank.
  [spec "a reranker ... the retrieve-then-rerank pattern the whole field uses"]
- [ ] A2.3 Backfill job: run the embedders over nodes on GPU (Modal), write
  vectors as the designated vector properties via `rustyred_thg_vector_designate`;
  RustyRed HNSW serves. A distinct channel from the GNN/KGE structural embeddings.
  [spec "How to get embeddings into the graph"]
- [ ] A2.4 Dimension contract: pin dim(s) (Qwen3-8B is MRL dimension-flexible);
  embed hot classes at full dim, compress cold/large classes. PUBLISH the
  contract (property name, dims, class->embedder map) to Lane B. [spec "The
  dimension flexibility matters ... vector-tiering ... at the embedder level"]
- [ ] A2.5 (spec-sequenced future, not a cut) Fine-tune on the substrate corpus
  via the CoRNStack methodology (hard-negative mining, consistency filtering).
  Baseline now, finetune later. [spec "adopt a current strong open embedder ...
  and then fine-tune it on the substrate's own corpus"]

### A3 The two inputs
- [ ] A3.1 Query path: query -> code search + fractal expansion (Lane B tool;
  existing Python fractal as fallback until B lands) -> grounded source ->
  encode -> export skill. [spec "Input one is a query"]
- [ ] A3.2 Upload path: a folder or ZIP up to ~1GB -> ingest (reuse the
  `code_repo` worker) -> embed -> encode -> skills distilled from their material.
  [spec "Input two is an upload"]
- [ ] A3.3 Upload lands in the uploader's TENANT, not the global substrate. [spec
  "The upload lands in the user's own tenant ... scoped to the uploader"]
- [ ] A3.4 Cap upload at ~1GB; meter / price on ENCODING COMPUTE, not storage.
  [spec "On the upload size cap and what actually costs money"]

### A4 Dual surface
- [ ] A4.1 Harness plugin surface: a skill Theorem uses internally to encode
  skills for its own use. [spec "internally, a skill Theorem uses"]
- [ ] A4.2 API/SPA surface: the external product (query box + upload). Honest
  empty states, no mock data, no demo route (project rules). [spec "externally,
  the API/SPA product"]

## Acceptance (the floor)

- A query through the LIVE product yields a downloadable Agent Skills folder
  whose scripts run in a vanilla compliant agent and whose provenance cites real
  ingested source.
- An uploaded repo yields tenant-scoped skills distilled from the user's own code.
- Embedder vectors are live in HNSW and served to Lane B's frontier.
