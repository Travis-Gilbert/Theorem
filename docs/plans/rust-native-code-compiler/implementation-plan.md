# Rust-Native Code Compiler Implementation Plan

## Decision

The Rust-native compiler extends `rustyred-thg-code` instead of introducing a parallel ingestion stack. Existing code ingest already owns the repository, file, symbol, call, and dependency graph shape; the compiler lowers that graph into durable compiler artifacts.

This keeps deterministic code in an infrastructure role: graph lowering, validation, artifact creation, and drift checks. Learned scoring remains the default product authority for semantic edges and competitive ranking.

## Ambient Principle

Prefer perception over tools for anything an agent should always be aware of. Code intelligence, epistemic standing, provenance, and related knowledge belong in the run-start memory scope, not only in the toolbelt. Tools remain for deliberate agent actions; compiler perception should be ambient.

For this compiler, the default product route is:

1. The compiler writes specs, drift, patterns, and features to the substrate.
2. Post-commit hooks keep cheap compiler artifacts current as code changes.
3. The existing `rustyred-thg-code` context-pack membrane folds the relevant compiler slice into the binding's memory scope at run start.

## Ten Stages

1. **Core IR and code-to-spec.** Built. Stable compiler inputs/outputs lower the current code graph into a `CodeSpecification`.
2. **Spec drift validation.** Built. Stored compiled spec snapshots are compared against current code graph state and drift findings are written.
3. **Ambient context projection.** Built. Spec/drift/process/pattern/feature/annotation readouts enter the existing code context pack.
4. **Reactive compiler hook.** Built. Graph post-commit hooks bootstrap/refresh compiler artifacts when code graph nodes and edges change.
5. **Process detector.** Built. Entrypoints and call flows lower from the existing symbol/call graph into `CodeProcessFlow`.
6. **Pattern memory.** Built. Fix patterns and positive feedback store as durable `CodePatternMemory` nodes.
7. **Feature extraction contract.** Built. The 21-signal lexical/BM25/SBERT/NLI/KGE/GNN/rule/novelty/time feature contract is stable; local infrastructure fills deterministic features and leaves model-owned signals importable.
8. **EDL/EBL annotation.** Built. Compiler feature records receive epistemic/aleatoric uncertainty and explanation/rule annotations.
9. **RunPod burst ingestion.** Built as contract/import path. Heavy parse/embed/analyze passes run outside the DB and import provenance-bearing artifacts back into the substrate.
10. **Harness ambient binding.** Built. The code context-pack membrane carries compiler perception, `coordination_context` and `harness_prepare` fold existing compiler artifacts into run-start context, and the receiver passes the job repo when it asks the harness for a launch packet.

## Stage 9 RunPod Evidence

- Worker image: `ghcr.io/travis-gilbert/theorem-code-compiler-worker:latest`, built by GitHub Actions run `28202383755` from commit `c33f2156`.
- Endpoint: RunPod serverless queue endpoint `theorem-code-compiler-burst` (`d95shhpxyk4nd2`), created on June 25, 2026 with the public GHCR image and no registry credentials.
- Live smoke: request `5a9c5239-0bca-4fc6-ab5b-e2dc081076de-u1` completed on worker `cncby7fb4nvili` with `11.56s` queue delay and `9.45s` execution time.
- Live learned output: the RunPod status response reported `calibration_version = runpod-edl-ebl-v1`, `active_feature_count = 12`, `evidence_count = 12`, `epistemic_uncertainty = 0.791667`, `aleatoric_uncertainty = 0.069191`, and explanation text naming learned embedding evidence from `sbert_cosine`, `bm25_score`, `spacetime_temporal_score`, `deep_analogy_score`, shared entities, notebook/object type, and cluster signals.
- Local contract smoke: `/tmp/code_compiler_runpod_worker_output.json` imports through `cargo run -p rustyred-thg-code --example import_code_compiler_runpod_response -- /tmp/code_compiler_runpod_worker_output.json`, producing an ambient readout with `feature_count = 2`, `annotation_count = 2`, `process_count = 2`, `artifact_count = 1`, and `bootstrapped_spec = true`.

## First Slice

- Add a `compiler` module to `rustyred-thg-code`.
- Compile a repository's current `CodeFile` and `CodeSymbol` graph into a `CodeSpecification` node.
- Store a canonical symbol snapshot on the spec node so later checks have an explicit oracle.
- Link the spec to covered symbols through `SPECIFIES_CODE` edges.
- Detect drift by comparing the compiled spec snapshot to the current graph:
  - missing symbol
  - undocumented symbol
  - signature changed
- Store drift findings as `CodeDriftFinding` nodes when callers choose the write-through helper.

## Later Slices

- Process detector lowering from the existing symbol/call graph.
- Pattern memory for fix patterns and positive feedback.
- RunPod burst-job contract for batch parsing, embeddings, complexity, history, and learned code features.
- EDL/EBL-backed annotation of compiler outputs once the learned feature stream is available.
- Adapter ingestion for SCIP, tree-sitter-graph, Joern CPG, and Docling-style artifacts without importing heavy dependencies at boot.

## Harness Exposure

Harness exposure is ambient by default. Agents should not need to remember a `compute_code` call to see known drift, relevant patterns, or compiled spec context for the repo they are about to touch. Explicit harness tools can still exist for deliberate deeper operations, but the normal run-start path places the relevant compiler readout in the binding-private/commons scope through the context-pack membrane and the MCP context surfaces.

Current ambient surfaces:

- `rustyred-thg-code` context packs include compiler spec, drift, process, pattern, feature, and annotation readouts.
- `rustyred-thg-mcp` `coordination_context` reads existing compiler artifacts by tenant/repo and returns both structured `ambient_code` and `ambient_code_markdown`.
- `rustyred-thg-mcp` `harness_prepare` renders the same compiler slice into the context brief when a repo is supplied.
- `theorem-receiver` passes the dispatch job repo into `coordination_context`, so launched heads inherit the compiler slice in their initial prompt packet.

## Acceptance

- `rustyred-thg-code` exposes stable compiler input/output structs.
- Tests prove spec compilation writes a spec node and symbol coverage edges.
- Tests prove drift detection reports missing, undocumented, and changed signatures from the stored spec snapshot.
- No product semantic edge is inferred from lexical heuristics in this compiler slice.
- Compiler intelligence is ambient in harness context by default, not only exposed as an opt-in tool.
- Harness context surfaces prove existing compiler artifacts are visible without calling `compute_code`.
