# Ghidra Reference Plan for the Theorem Engineering Compiler

This is not a plan to port Ghidra wholesale, and Ghidra is not treated as a teacher-agent. Ghidra is a primary reference, oracle source, fixture source, and architecture vocabulary for Theorem's engineering compiler.

Theorem should own the work product:

```text
ProgramAnalysisRun
  -> EvidenceMap
  -> EngineeringSpec
  -> ImplementationObligation
  -> ValidatorSpec
  -> agent patch/review/test receipts
```

Ghidra can help define the binary-analysis branch of that pipeline, but the same compiler must also ingest web behavior, source-code graphs, runtime traces, docs, issues, PRs, and pattern memory.

## Primary References

- Ghidra README: https://github.com/NationalSecurityAgency/ghidra
- Headless analyzer: https://github.com/NationalSecurityAgency/ghidra/blob/master/Ghidra/RuntimeScripts/support/analyzeHeadlessREADME.md
- P-code reference: https://github.com/NationalSecurityAgency/ghidra/blob/master/GhidraDocs/languages/html/pcoderef.html
- SLEIGH reference: https://github.com/NationalSecurityAgency/ghidra/blob/master/GhidraDocs/languages/html/sleigh.html
- Analyzer extension skeleton: https://github.com/NationalSecurityAgency/ghidra/blob/master/GhidraBuild/Skeleton/src/main/java/skeleton/SkeletonAnalyzer.java

## What To Take First

1. Headless analysis contract
   - Ghidra's `analyzeHeadless` shape is the right oracle model: import/process a binary, run non-GUI scripts, configure language/compiler spec, bound analysis time, and emit logs.
   - Theorem equivalent: `ProgramAnalysisRun` with explicit target, toolchain, analyzer profile, timeout, artifact hashes, logs, and receipts.

2. Retargetable IR principle
   - Ghidra's p-code exists because processor-specific instructions need a common semantic layer for data-flow and control-flow analysis.
   - Theorem equivalent: do not jump from disassembly to reconstruction obligations. Lower into a small Theorem IR first, with explicit address spaces, varnodes/registers, operations, branches, calls, loads, stores, and returns.

3. SLEIGH boundary, not SLEIGH port
   - SLEIGH is a processor-specification language for translating machine instructions into p-code.
   - Theorem should not start by porting SLEIGH. The first Theorem path should use existing Rust disassembly/lift libraries for a narrow target, then model a SLEIGH-compatible vocabulary so later oracle parity is possible.

4. Analyzer plugin shape
   - Ghidra analyzers have a small shape: `canAnalyze`, default enablement/options, and an `added` analysis pass over a program/address set.
   - Theorem equivalent: analyzer passes are graph workers over a scoped `ProgramAnalysisRun`; each pass declares input labels, output labels, authority layer, and validator receipts.

5. Oracle fixture strategy
   - Use Ghidra to generate expected facts for small binaries: sections, imports, symbols, functions, CFG edges, p-code-like operation sequences, strings, and decompiler-independent data-flow facts.
   - Theorem's Rust pipeline passes only when its graph facts match the oracle fixture within an explicitly allowed delta.

## What Not To Port Now

- The GUI, project manager, docking UI, graph viewers, and analyst workflow chrome.
- The full decompiler.
- The full processor zoo.
- The full SLEIGH compiler.
- Years of architecture-specific analyzers.
- Human-readable decompile output as the product surface.

The product target is not "show me decompiled C." It is "compile evidence-backed construction obligations an agent can implement and validators can check."

## Theorem Pipeline Shape

```text
BinaryArtifact
  -> LoaderFacts
  -> InstructionFacts
  -> TheoremIR
  -> ProgramGraphFacts
  -> ProgramAnalysisRun
  -> EngineeringCompileInput
  -> EngineeringCompileOutput
```

The binary branch should join the existing non-binary compiler inputs:

```text
web behavior observations
code KG/spec/drift/process/pattern artifacts
runtime trace contracts
docs/issues/PR evidence
pattern memory
binary ProgramAnalysisRun facts
  -> EngineeringCompileInput
  -> obligations + validators
```

## Data Contracts

ProgramAnalysisRun:
- `tenant_id`
- `artifact_id`
- `target_kind`: `binary|repo|site|feature|api`
- `toolchain`: analyzer versions and external oracle versions
- `profile`: selected analyzers
- `started_at_ms`, `finished_at_ms`
- `status`: `pending|running|complete|failed|partial`
- `artifact_hash`
- `receipt_hash`

Binary facts:
- `BinaryArtifact`: sha256, format, arch, endian, entrypoints, load base
- `LoaderFact`: sections, segments, relocations, imports, exports, strings
- `InstructionFact`: address, bytes hash, mnemonic, operands, fallthrough, branch target
- `TheoremIrFunction`: function id, address range, blocks, statements
- `ProgramDataFlowFact`: def/use, memory read/write, call edge, return value observation
- `ProgramSemanticHypothesis`: role, confidence, model id, evidence ids

Engineering outputs:
- `EvidenceMap`
- `BehaviorSpec`
- `ArchitectureMap`
- `ApiContract`
- `ImplementationObligation`
- `ValidatorSpec`
- `UnknownsLedger`

## Acceptance Tests

1. Headless oracle fixture
   - Run Ghidra headless on a tiny C fixture.
   - Export JSON facts through a post-script.
   - Theorem loader/disasm/lift emits matching artifact hash, entrypoint, function count, import names, and CFG edges.

2. P-code vocabulary parity
   - For a tiny instruction sequence, Ghidra p-code export and Theorem IR agree on operation categories: load, store, call, branch, conditional branch, return.
   - Exact opcode parity is required only for operations Theorem claims to support.

3. Analyzer pass receipts
   - Every Theorem analyzer pass writes graph nodes with `tenant_id`, `artifact_id`, `run_id`, `authority_layer`, and `evidence_ids`.
   - Re-running a pass is idempotent by `(run_id, analyzer_id, input_hash)`.

4. Obligation compilation
   - Binary evidence plus code/web/trace evidence compiles into an obligation with validators and unknowns.
   - A generated obligation never cites raw disassembly alone; it cites bounded evidence ids.

5. No GUI/decompiler dependency
   - Theorem binary analysis tests run in CI without launching a GUI.
   - Ghidra is used only in explicit oracle-generation jobs, not in the default product path.

## Build Order

1. Add `ProgramAnalysisRun` contracts and graph labels.
2. Add oracle fixture schema for Ghidra headless JSON exports.
3. Add minimal binary fixture and Ghidra post-script exporter.
4. Add Theorem loader/disasm fixture reader for the same binary.
5. Add minimal Theorem IR vocabulary aligned with p-code concepts.
6. Add parity tests against fixture JSON.
7. Feed binary facts into `EngineeringCompileInput`.
8. Compile a mixed evidence obligation and validator.

## Design Rule

Prefer perception over tools. Theorem should produce the evidence map and obligations automatically when a target enters scope. Codex or Claude should review, patch, and test against the validators; they should not have to remember to manually ask for raw reverse engineering first.
