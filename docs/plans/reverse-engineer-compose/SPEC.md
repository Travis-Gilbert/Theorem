# SPEC: Unify the reverse-engineer pipeline and ship the compose verb

**Status:** proposed
**Date:** 2026-06-29
**Owner:** TBD
**Acceptance test:** `theorem.reverse_engineer.compose("https://github.com/mindsdb/lightwood.git")` returns a `ReconstructionSpec` JSON shaped per section 5, without the agent doing any `git clone` / `rg` / `Read` of its own.

## 0. Why this spec exists

The reverse-engineer skill today produces an evidence map and a rebuild plan by having the agent run `git clone --depth 1` + `rg` + `Read` directly. The pieces of an end-to-end pipeline that would do this with built engine surface are partially built and partially wired:

| Lane | Pub Rust | MCP-exposed | Output node types |
|------|----------|-------------|-------------------|
| Source repo ingest | `rustyred-thg-code::ensure::ensure_repo_kg` | yes (`code_ingest`) | `CodeRepository`, `CodeFile`, `CodeSymbol`, `CALLS_SYMBOL`, `DEPENDS_ON_SYMBOL` |
| Code search / read | `rustyred-thg-code` (CodeCrawler) | yes (`compute_code`) | reads above |
| Binary intake | `rustyred-thg-binformat` -> `disasm` -> `lift` -> `reconstruct` | yes (`reconstruct_binary`) | `BinaryArtifact`, `BinarySection`, `BinarySymbol`, `ReconstructionInstruction` |
| Universal record intake | `rustyred-thg-datawave::DatawaveIngest` | yes (`datawave_ingest`) | `IngestEvent`, `FieldFact`, `FieldEntity` |
| Web/page intake | `rustyred-web::web_consume_to_graph` | yes (`web_consume` / `rustyweb_search_acquisition`) | `Page`, `Phrase` |
| **Code compiler** (the spec extractor) | `rustyred-thg-code/src/compiler/*` (`compile_code_spec`, `extract_code_features`, `compile_code_implementation_obligations`, `relevant_code_patterns`, `detect_code_spec_drift`) | **no** | reads code-lane types; writes `CodeSpecification`, `CodeFeatureRecord`, `CodeImplementationObligation`, `CodeSpecDriftFinding`, `CodePatternMemoryRecord` |
| **Compose** (spec -> source in target context) | **does not exist** | **no** | n/a |

Two real problems flow from this:

1. The compiler lane and the compose verb have no agent surface, so the skill cannot reach them and falls back to manual file reading.
2. Each ingest lane writes a disjoint node-type set. The plan docs at `docs/plans/datawave-ingest-edge/STATUS.md:11` claim "they compose because they share the GraphStore," but sharing a substrate is not the same as sharing a schema. Nothing today joins datawave's `FieldFact` view of a binary to reconstruction's `BinaryArtifact` view of the same binary, or to the compiler's `CodeSpecification` view of source.

## 1. Scope

Ship a single new MCP capability pack `theorem.code.compose` plus three projection bridges. Endpoint:

```
theorem.reverse_engineer.compose(
  source: SourceRef,            // GitHub URL, local path, binary path, web URL
  target: Option<TargetContext> // None = just return the spec, do not emit code
) -> ReconstructionSpec
```

Out of scope for this spec (named follow-ups, not deferrals): emitter for arbitrary target contexts beyond a baseline Python emitter; pattern transfer beyond direct re-use; multi-language source emission.

## 2. The pipeline (post-unification)

```
        SourceRef
            |
            v
   +--------+--------+--------+--------+
   |        |        |        |        |
   v        v        v        v        v
code_ingest reconstruct_binary  datawave_ingest  web_consume
   |        |        |        |
   v        v        v        v
CodeFile  BinaryArt FieldFact  Page
CodeSym   BinarySym FieldEnt   Phrase
            |
            v
  +---------+----------+
  | projection bridges |  <-- the three new modules in section 4
  +---------+----------+
            |
            v
     unified spec corpus (CodeSpecification + CodeImplementationObligation
     + CodeFeatureRecord + relevant CodePatternMemoryRecord + optional
     ReconstructionInstruction)
            |
            v
       compose(spec, target)  --(target=None)--> ReconstructionSpec JSON
                              --(target=Ctx)--> emitted source in target
```

The pipeline is parallel intake then serial compose, not serial intake.

## 3. MCP surface to add

Add to `rustyred-thg-mcp/src/lib.rs` (follow the existing pattern from `code_ingest` / `compute_code` registration; mirror the four-site checklist in `docs/learnings/2026-06-07-adding-mcp-verb-family-to-harness.md`).

| MCP tool name | Wraps | Read / write |
|---------------|-------|--------------|
| `code_compile_spec` | `compiler::compile_code_spec_in_store` | read |
| `code_extract_features` | `compiler::extract_code_features_in_store` | read |
| `code_implementation_obligations` | `compiler::compile_code_implementation_obligations_in_store` | read |
| `code_patterns_relevant` | `compiler::relevant_code_patterns` | read |
| `code_spec_drift` | `compiler::detect_code_spec_drift_in_store` | read |
| `reverse_engineer_compose` | new function (section 6) | read+write (writes the receipt) |

All inputs accept `tenant_id` + a record key (`repo_id` / `spec_id` / `source_ref`). All outputs are the structured types already defined in `rustyred-thg-code/src/compiler/ir.rs`. No new IR.

## 4. Projection bridges (the schema work)

Three bridge modules. **All three ship**, because the user's pipeline (compute_code -> datawave -> ghidra/compiler -> spec) only works if every adjacent pair speaks both directions.

### 4.1. `rustyred-thg-code-to-datawave`

New crate. Maps `CodeFile` / `CodeSymbol` to `FieldFact` / `FieldEntity` so a code repo also intersects in the universal record index.

Inputs:
- `tenant_id: String`
- `repo_id: String`
- (read from store)

Effect:
- For each `CodeFile`, emit `FieldFact(field="file_path", value=path)`, `FieldFact(field="language", value=lang)`, `FieldFact(field="content_hash", value=hash)`. The `vf="file_path=…"` property rides datawave's existing property index (`materialize.rs`), so a cross-source `lookup` finds the same hash a `datawave_ingest` of a CSV pointing at the same file would.
- For each `CodeSymbol`, emit `FieldFact(field="symbol_name", value=name)` and a declared `FieldEntity` link to its parent `CodeFile`'s file-path fact.
- Write a `DatawaveParityReceipt` and a `LabeledTrainingRun` entry per the existing `training.rs` stream so retrieval training sees it.

Pub function: `project_code_to_datawave(store, &input) -> Result<ProjectionReceipt>`.

### 4.2. `rustyred-thg-datawave-to-code`

New crate. The inverse: takes a `MappedHelper` source contract describing a code-like record stream and emits `CodeFile` / `CodeSymbol` nodes via the existing `CodeIndexRuntime::ingest_codebase_from_url`-style write path (or its in-store equivalent).

This is what closes "compute_code's ingestion belongs in front of datawave" by also accepting "datawave's ingestion can produce code-lane nodes." Either direction works.

### 4.3. `rustyred-thg-binformat-to-datawave`

Lifts the `BinaryArtifact` / `BinarySection` / `BinarySymbol` / `ReconstructionInstruction` set into `FieldFact` form. Binary `import` / `relocation` / `string` records become field-facts; cross-binary intersection then uses the same property-index path as everything else.

Use the existing `rustyred-thg-datawave::hash::content_hash` and `fuzzy_hash` so binary similarity (the design intent named in the STATUS.md sync line) actually fires.

### 4.4. Where the bridges live in the pipeline

The bridges run as **post-ingest hooks** wired through `rustyred-thg-core`'s post-commit hook system (the same one `incremental_code_compiler_hook` already uses). A `code_ingest` writes `CodeFile` / `CodeSymbol`, the hook fires `project_code_to_datawave`, the `FieldFact`s land in the same transaction generation. No new ordering rules.

## 5. The compose function

New module: `rustyred-thg-code/src/compose/mod.rs`.

Pub function:

```rust
pub fn compose_reconstruction_spec_in_store<S: GraphStore>(
    store: &S,
    input: &ComposeInput,
) -> GraphStoreResult<ReconstructionSpec>;
```

Where `ReconstructionSpec` is:

```rust
pub struct ReconstructionSpec {
    pub source_ref: SourceRef,             // echo of input
    pub code_spec: Option<CodeSpecCompileOutput>,
    pub features: Vec<CodeFeatureRecord>,
    pub obligations: Vec<CodeImplementationObligation>,
    pub patterns: Vec<CodePatternMemoryRecord>,
    pub binary: Option<BinaryReconstructionSummary>,
    pub datawave_facts: Vec<FieldFactSummary>, // joined via property index
    pub drift: Vec<CodeSpecDriftFinding>,
    pub provenance: ComposeProvenance,
}
```

Body: call the five existing compiler verbs in order (`compile_code_spec` -> `extract_code_features` -> `compile_code_implementation_obligations` -> `relevant_code_patterns` -> `detect_code_spec_drift`), plus the binary summary if `BinaryArtifact` nodes exist under the source_ref, plus a property-index pull for `FieldFact`s tagged with the source's `repo_id` / `content_hash`. No new analysis; this is pure assembly.

For `target = Some(...)`: a follow-on emitter takes the spec + target context and writes source. The lightwood miniature in `scratchpad/feature_codegen_demo.py` is the template for the source-source case. Out of this spec; gated.

## 6. Acceptance test

One test, runnable via the MCP layer with no other agent help.

**Cold path** (repo unknown to the graph):

```
call: reverse_engineer_compose({"source": {"github_url": "https://github.com/mindsdb/lightwood.git"}})

expect: HTTP 200
expect: result.code_spec.code_compiler_version == "rustyred-code-compiler-v0"
expect: result.code_spec.spec.label == "CodeSpecification"
expect: result.code_files_count >= 200            # lightwood has 248 tracked
expect: result.code_symbols_count >= 500          # real codebase
expect: result.obligations.len() >= 1
expect: result.patterns is an array (may be empty on a cold corpus)
expect: result.binary == null                     # source-only target
expect: result.datawave_facts.len() >= result.code_files_count
                                                  # the bridge emitted file-path facts
expect: result.drift.len() == 0                   # nothing to drift against on first ingest
expect: result.provenance.ingest_path == "FullyIngested"
```

**Warm path** (same SHA, second call):

```
call: reverse_engineer_compose({"source": {"github_url": "https://github.com/mindsdb/lightwood.git"}})

expect: result.provenance.ingest_path == "LoadedFromSnapshot"
expect: result.code_spec.spec.id == <same as cold call>
expect: latency < 0.1 * cold_call_latency        // proves the snapshot path fired
```

**Datawave cross-source join** (proves the bridge):

```
ingest_csv_pointing_at_lightwood_file_paths()    // any CSV with a file_path column
call: datawave_ingest_intersect({"field": "file_path", "value": "<some lightwood file>"})

expect: intersect returns at least one FieldFact whose source is the code lane
        AND one whose source is the CSV lane
```

These three pass = the unification works end-to-end. They fail = either MCP exposure (3) is incomplete or the bridges (4) didn't fire.

## 7. Order of work

1. Section 3 only. Wire the five existing compiler verbs to MCP, including dispatch + schema + read-only tools/list assertion in `lib.rs`. Smallest scoped change; immediately makes the reverse-engineer skill stop hand-rolling rg.
2. Section 5. Write `compose_reconstruction_spec_in_store` over the now-exposed verbs. Test cold-path assertion subset (no datawave join yet).
3. Section 4.1. `rustyred-thg-code-to-datawave` projection + hook wiring. Test datawave-cross-source-join assertion.
4. Section 5 wraps the new field-fact view into `ReconstructionSpec.datawave_facts`.
5. Section 4.3 (binary projection). Add `result.binary` content to the warm-path assertion when fed a binary URL.
6. Section 4.2 (inverse projection). Validates that `datawave_ingest` of a code-shaped record produces the same `CodeFile` / `CodeSymbol` nodes a direct `code_ingest` would. Test asserts node-set equivalence on a small fixture.

Each step is independently shippable and independently testable. None of them are interleaved.

## 8. References

- `rustyred-thg-mcp/src/lib.rs:17249-17260` (current exposed tool set)
- `rustyred-thg-code/src/compiler/{ir.rs,code_to_spec.rs,features.rs,obligations.rs,pattern.rs,drift.rs,hooks.rs}` (compiler lane to expose)
- `rustyred-thg-code/src/ensure.rs` (`ensure_repo_kg` / `RepoKgStatus` for the warm-path receipt)
- `rustyred-thg-datawave/src/{materialize.rs,training.rs,hash.rs}` (datawave write path the bridges reuse)
- `rustyred-thg-datawave-harness/src/lib.rs:30` (capability pack template)
- `docs/plans/datawave-ingest-edge/STATUS.md` (the original unification claim this spec makes real)
- `docs/learnings/2026-06-07-adding-mcp-verb-family-to-harness.md` (four-site checklist for section 3)
- `scratchpad/feature_codegen_demo.py` (working miniature of the source-emitter path, for the later `target = Some(...)` work)
