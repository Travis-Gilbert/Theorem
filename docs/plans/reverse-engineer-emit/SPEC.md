# SPEC: Feature-port emission on top of reverse-engineer compose

**Status:** first vertical implemented
**Date:** 2026-06-29
**Owner:** TBD
**Depends on:** `docs/plans/reverse-engineer-compose/SPEC.md`
**Goal:** turn an engine-produced `ReconstructionSpec` into target-language code, tests, and a parity report.

**Implementation note (2026-06-29):** the first vertical landed in
`rustyred-thg-behavior-ir`, `rustyred-thg-code`, and `rustyred-thg-mcp`.
The MCP surface now exposes `reverse_engineer_slice`,
`reverse_engineer_behavior_ir`, `reverse_engineer_target_plan`,
`reverse_engineer_emit`, `reverse_engineer_validate`, and
`reverse_engineer_port` on top of the compose/ingest front door. The current
emitters produce conservative TypeScript/Rust API surfaces, operation metadata,
validation receipts, and explicit unresolved obligations. The first semantic
lowering preserves source bodies in `BehaviorIr` and translates simple numeric
`return` expressions, local numeric assignment plus return bodies, and
guarded-return chains into TypeScript/Rust operation bodies; unsupported
operation bodies still return review-required errors instead of inventing
behavior. For translated numeric operations, the emitter also generates concrete
example assertions in the target test scaffold.

## 0. Why this spec exists

`reverse_engineer_compose` now proves the first half of the intended tool:

```
repo URL -> code graph -> CodeSpecification / features / obligations / Datawave facts
```

The product value is the second half:

```
ReconstructionSpec + target context -> emitted patch + tests + validation
```

Lightwood and Rust are only fixtures. The architecture must generalize across
popular source and target languages, starting with a small proof path and then
expanding through language plugins.

## 1. Reference repos and the shapes to borrow

### 1.1. OpenRewrite: recipe, visitor, result

Repository: <https://github.com/openrewrite/rewrite>

Observed shape:

- `Recipe` owns configuration, metadata, examples, and the `run(...)` loop.
- `Recipe.getVisitor()` returns a typed tree visitor.
- `TreeVisitor` traverses and rewrites source trees.
- `Result` captures before/after source files and the recipes that changed them.
- Java recipes use typed visitors such as `JavaIsoVisitor` and templates such as
  `JavaTemplate`.

Borrow:

- A `PortRecipe` abstraction that is explicit, named, and replayable.
- A `PatchResult` shape with before/after artifacts and provenance.
- Typed target-language visitors/templates rather than string-only emitters.

Do not borrow blindly:

- OpenRewrite is strongest in Java and ecosystem migrations. Theorem needs a
  cross-language behavior IR before it becomes a recipe engine.

### 1.2. LibCST: lossless Python transforms and codemods

Repository: <https://github.com/Instagram/LibCST>

Observed shape:

- `CSTTransformer` and `CSTVisitor` preserve concrete syntax.
- `CodemodCommand` runs transformations with command context.
- `VisitorBasedCodemodCommand` combines visitor transforms with CLI/codemod
  execution.

Borrow:

- Lossless CST discipline for source edits where the source language is also the
  target language or where we patch an existing target project.
- A codemod runner model that can apply generated transformations safely.

Do not borrow blindly:

- LibCST is Python-specific. It is a target-adapter pattern, not the shared core.

### 1.3. CPG / Joern / Fraunhofer CPG: graph frontend

Repositories:

- <https://github.com/joernio/joern>
- <https://github.com/Fraunhofer-AISEC/cpg>

Observed shape:

- Code is projected into a labelled directed graph.
- The graph supports cross-language analysis over source, bytecode, and binaries.
- Frontends add language-specific detail; passes enrich the graph after initial
  parse.

Borrow:

- Theorem's code graph should keep moving toward a CPG-like shared schema:
  declarations, calls, data flow, control flow, imports, types, tests, docs, and
  runtime observations.
- Language adapters should populate the shared graph plus language-specific
  extension facts.

Do not borrow blindly:

- CPGs are analysis substrates. They do not, by themselves, emit idiomatic target
  code.

### 1.4. MLIR: dialects and lowering passes

References:

- <https://github.com/llvm/llvm-project>
- <https://mlir.llvm.org/>

Observed shape:

- Multiple dialects coexist.
- Passes progressively lower from higher-level dialects to lower-level dialects.
- Target-specific lowering is explicit.

Borrow:

- Model the behavior IR as dialects:
  - `behavior.core`
  - `behavior.data`
  - `behavior.effects`
  - `behavior.tests`
  - `target.rust`, `target.ts`, `target.java`, etc.
- Make lowering passes explicit and testable.

Do not borrow blindly:

- Do not make Theorem a compiler framework clone. Use the dialect/pass idea,
  not necessarily MLIR's exact runtime.

### 1.5. py2many: many target emitters

Repository: <https://github.com/py2many/py2many>

Observed shape:

- A shared CLI runs analysis/rewriters/transforms, then calls a language-specific
  transpiler.
- Target emitters implement visitor methods such as `visit_FunctionDef`,
  `visit_Call`, `visit_For`, `visit_ClassDef`, etc.
- Targets include Rust and C++ emitters in separate modules.

Borrow:

- A plugin layout where each target emitter is its own module/crate.
- The notion of shared analysis plus target-specific emission.

Do not borrow blindly:

- py2many starts from Python AST and emits language syntax. Theorem needs a
  behavior IR, not a Python-AST-as-universal-IR.

### 1.6. m2cgen: compact IR to many languages

Repository: <https://github.com/BayesWitnesses/m2cgen>

Observed shape:

- A compact model representation is interpreted into many target languages.
- It supports many languages, including C, Java, Go, JavaScript, Python, Ruby,
  Rust, and others.
- The repo separates model assembly from target-language interpretation.

Borrow:

- Keep the shared IR small and semantically loaded.
- Target emitters should interpret a portable contract, not raw source syntax.
- Generated output must carry numeric/semantic parity caveats where target
  runtimes differ.

Do not borrow blindly:

- m2cgen handles constrained statistical model expressions. General feature
  porting needs effects, dependencies, tests, and project integration.

### 1.7. c2rust: preservation first, idiom later

Repository: <https://github.com/immunant/c2rust>

Observed shape:

- The translator preserves functionality first.
- Initial Rust may be unsafe/non-idiomatic.
- Test suites are the oracle; idiomatic cleanup is a later phase.

Borrow:

- Acceptance should be parity-first, not beauty-first.
- Emission can have stages: faithful port, cleanup obligations, idiomatic
  refactor recipe.

Do not borrow blindly:

- C-to-Rust has unusually concrete semantics and compile commands. Dynamic
  languages and framework features need traces/examples/tests to fill gaps.

### 1.8. Ghidra / RetDec: binary fallback, not normal source path

Repositories:

- <https://github.com/NationalSecurityAgency/ghidra>
- <https://github.com/avast/retdec>

Observed shape:

- Binary analysis and decompilation are valuable when source is missing or the
  binary is the authority.
- RetDec is LLVM-based; Ghidra provides disassembly, decompilation, scripting,
  graphing, and broad processor support.

Borrow:

- Keep `reconstruct_binary` as a frontend into the same behavior IR.
- Binary facts should join with source/datawave facts when both exist.

Do not borrow blindly:

- Do not compile source to binary just to decompile it for feature porting.
  Source is richer evidence than a compiled artifact.

## 2. Proposed Theorem architecture

```
SourceRef
  -> intake adapters
       code_ingest / reconstruct_binary / datawave_ingest / web_consume
  -> graph projections
       CodeFile, CodeSymbol, BinaryArtifact, FieldFact, Page, Trace
  -> behavior extraction
       FeatureSlice -> BehaviorIr
  -> target lowering
       BehaviorIr -> TargetPlan
  -> target emitter
       TargetPlan -> PatchSet + Tests
  -> validation
       parity tests + compile/test receipts + obligations
```

This is not a universal AST. It is a portable behavior contract.

## 3. Core data types

### 3.1. `FeatureSlice`

The selected source feature and evidence boundary:

```rust
pub struct FeatureSlice {
    pub slice_id: String,
    pub source_ref: SourceRef,
    pub tenant_id: String,
    pub repo_id: String,
    pub entry_symbols: Vec<String>,
    pub files: Vec<String>,
    pub tests: Vec<String>,
    pub docs: Vec<String>,
    pub runtime_examples: Vec<String>,
    pub dependencies: Vec<DependencyRef>,
    pub unknowns: Vec<String>,
}
```

Extraction sources:

- explicit user seed: "port Lightwood's JSON AI codegen"
- `CodeSpecification` symbols and dependency edges
- PPR around seed symbols via `compute_code`
- tests/docs/examples via code graph and Datawave facts

### 3.2. `BehaviorIr`

The portable contract:

```rust
pub struct BehaviorIr {
    pub ir_id: String,
    pub feature: FeatureSlice,
    pub purpose: String,
    pub public_api: Vec<ApiContract>,
    pub data_models: Vec<DataModelContract>,
    pub operations: Vec<OperationContract>,
    pub control_flow: Vec<ControlFlowContract>,
    pub effects: Vec<EffectContract>,
    pub errors: Vec<ErrorContract>,
    pub examples: Vec<ExampleContract>,
    pub tests: Vec<TestContract>,
    pub invariants: Vec<InvariantContract>,
    pub portability_hazards: Vec<PortabilityHazard>,
    pub evidence: Vec<EvidenceRef>,
}
```

Initial operation dialect:

- pure expression
- stateful object method
- parser/serializer
- model wrapper
- HTTP/client boundary
- file/db/network effect
- async/concurrency boundary
- unsafe/native/binary boundary

### 3.3. `TargetPlan`

Language and project-specific lowering:

```rust
pub struct TargetPlan {
    pub target_language: TargetLanguage,
    pub target_project: TargetProjectContext,
    pub module_plan: Vec<TargetModulePlan>,
    pub dependency_substitutions: Vec<DependencySubstitution>,
    pub idiom_level: IdiomLevel, // faithful, idiomatic, framework_native
    pub validation_plan: ValidationPlan,
    pub obligations: Vec<TargetObligation>,
}
```

### 3.4. `PatchSet`

OpenRewrite-inspired result:

```rust
pub struct PatchSet {
    pub patch_id: String,
    pub target_language: TargetLanguage,
    pub files: Vec<PatchFile>,
    pub tests: Vec<PatchFile>,
    pub receipts: Vec<ValidationReceipt>,
    pub unresolved_obligations: Vec<TargetObligation>,
}
```

## 4. MCP surface

Add these tools after `reverse_engineer_compose`:

| Tool | Purpose |
| --- | --- |
| `reverse_engineer_slice` | select a feature boundary from source evidence |
| `reverse_engineer_behavior_ir` | lower `ReconstructionSpec + FeatureSlice` into `BehaviorIr` |
| `reverse_engineer_target_plan` | lower `BehaviorIr` into a target-language/project plan |
| `reverse_engineer_emit` | produce patch files and tests |
| `reverse_engineer_validate` | run or compile parity checks and write receipts |

Convenience:

| Tool | Purpose |
| --- | --- |
| `reverse_engineer_port` | full pipeline: compose -> slice -> IR -> target plan -> emit -> validate |

## 5. Language plugin layout

Start with plugin crates:

```
rustyred-thg-behavior-ir
rustyred-thg-feature-port
rustyred-thg-port-python
rustyred-thg-port-typescript
rustyred-thg-port-rust
```

Then expand:

```
rustyred-thg-port-java
rustyred-thg-port-cpp
rustyred-thg-port-c
rustyred-thg-port-ruby
rustyred-thg-port-go
rustyred-thg-port-csharp
```

Each language plugin should expose:

```rust
pub trait SourceFrontend {
    fn extract_behavior(&self, slice: &FeatureSlice, graph: &dyn GraphStore) -> BehaviorIr;
}

pub trait TargetEmitter {
    fn plan(&self, ir: &BehaviorIr, context: &TargetProjectContext) -> TargetPlan;
    fn emit(&self, ir: &BehaviorIr, plan: &TargetPlan) -> PatchSet;
}
```

## 6. Compute-code role

`compute_code` is useful, but not as the emitter. Its job is feature boundary
selection and evidence retrieval.

Use it for:

- PPR around seed symbols/files.
- Finding tests/docs/examples structurally near the feature.
- Explaining why a dependency or file is in the slice.
- Ranking candidate entrypoints.
- Rechecking unknowns when grep misses related code.

Do not use it for:

- Code generation.
- Translation semantics.
- Target project patching.

In this planning pass, the remote `_compute_code` call exceeded the MCP HTTP
budget, so examples were pulled directly from GitHub. That is a runtime surface
issue, not a reason to remove `compute_code` from the design.

## 7. First executable slice

Build the narrowest valuable proof:

```
Python source feature -> BehaviorIr -> TypeScript emitter + tests
Python source feature -> BehaviorIr -> Rust emitter + tests
```

Use Lightwood only as the second fixture. The first fixture should be smaller:

- one parser/serializer feature
- one pure algorithm feature
- one stateful class/object feature

Acceptance:

1. `reverse_engineer_slice` selects files, symbols, tests, docs, and dependencies
   from a user seed.
2. `reverse_engineer_behavior_ir` emits a stable JSON IR with examples and
   unknowns.
3. `reverse_engineer_emit(target=typescript)` creates a compilable patch and
   tests.
4. `reverse_engineer_emit(target=rust)` creates a compilable patch and tests.
5. `reverse_engineer_validate` passes generated parity tests or returns explicit
   unresolved obligations.

## 8. Implementation order

### Step 1: Behavior IR crate

- Add `rustyred-thg-behavior-ir`.
- Define `FeatureSlice`, `BehaviorIr`, `TargetPlan`, `PatchSet`.
- Add serde JSON golden tests.

### Step 2: Slice selection

- Add `reverse_engineer_slice`.
- Use current `ReconstructionSpec` plus graph edges.
- Wire optional `compute_code` evidence when available.
- Test with a small local fixture.

### Step 3: Python frontend

- Extract API contracts, dataclasses/classes, functions, examples, and tests.
- Prefer AST/CST-based extraction when source text is available.
- Record unknown dynamic behavior as obligations instead of pretending.

### Step 4: TypeScript emitter

- Emit modules, interfaces/types, pure functions, simple classes, tests.
- Use templates/AST printer, not raw concatenation.
- Validate with `tsc` or project test command when available.

### Step 5: Rust emitter

- Emit crate/module files, structs/enums/functions, and tests.
- Start with faithful behavior, not maximally idiomatic Rust.
- Validate with `cargo check` and generated tests.

### Step 6: Patch result model

- OpenRewrite-inspired before/after/result receipts.
- Include provenance and unresolved obligations.
- Add MCP output schema.

### Step 7: Popular language expansion

Priority order:

1. Python
2. TypeScript / JavaScript
3. Rust
4. Java
5. Go
6. C++
7. C
8. Ruby
9. C#

The order can change when real customer/project fixtures demand it.

## 9. Validation philosophy

Borrow from c2rust: functionality preservation is the first gate.

Every emitted port needs at least one oracle:

- original test translated to target test
- generated example parity test
- trace replay
- property/invariant test
- compile/typecheck receipt

An output with unresolved obligations is acceptable if it is explicit:

```json
{
  "status": "needs_review",
  "patch": [...],
  "passed": ["cargo check"],
  "unresolved_obligations": [
    "Python dynamic import cannot be mapped without target dependency choice"
  ]
}
```

An output that silently invents behavior is not acceptable.

## 10. Open questions

- Should `BehaviorIr` live in `rustyred-thg-code` initially or a new crate?
- Should target emitters write files directly or only return patches?
- Should Theorem use tree-sitter for all source frontends, or language-native
  ASTs where they are better?
- What is the first non-Lightwood fixture?
- Should target-language style be learned from the destination project before
  emission?

## 11. Links

- OpenRewrite: <https://github.com/openrewrite/rewrite>
- LibCST: <https://github.com/Instagram/LibCST>
- Joern: <https://github.com/joernio/joern>
- Fraunhofer CPG: <https://github.com/Fraunhofer-AISEC/cpg>
- MLIR: <https://mlir.llvm.org/>
- py2many: <https://github.com/py2many/py2many>
- m2cgen: <https://github.com/BayesWitnesses/m2cgen>
- c2rust: <https://github.com/immunant/c2rust>
- Ghidra: <https://github.com/NationalSecurityAgency/ghidra>
- RetDec: <https://github.com/avast/retdec>
