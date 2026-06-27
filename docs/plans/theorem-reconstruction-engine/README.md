# Theorem Reconstruction Engine

## Goal

Compile a binary artifact into versioned GraphStore facts and bounded reconstruction obligations. The engine is not a human decompiler surface. Agents receive `ReconstructionInstruction` nodes with evidence and validators, then write validation receipts after rebuilding equivalent behavior.

## Current Native Slice

- `rustyred-thg-binformat`: parses object files with `object`, extracts artifact, section, symbol, string, relocation, and entrypoint fact models, and writes observed facts to GraphStore.
- `rustyred-thg-disasm`: decodes x86-64 executable sections with `iced-x86` and emits `InstructionFact` nodes with flow-control, branch targets, bytes, and coarse effects.
- `rustyred-thg-lift`: lowers instruction facts into minimal THIR functions, basic blocks, and SSA-like statements.
- `rustyred-thg-reconstruct`: derives basic semantic-role hypotheses, recovers component hypotheses, compiles evidence-backed reconstruction instructions, and records validation receipts.
- `rustyred-thg-reconstruct-harness`: exposes the `theorem.reconstruct.binary` capability pack and plugin operations for load, analyze, lift, graph write, component recovery, plan compile, instruction get, validation, and receipt write.
- `rustyred-thg-mcp`: exposes the agent-facing `reconstruct_binary` umbrella tool with operation selection and read-only gating.

## Authority Layers

- `observed_fact`: loader, decoder, and lifter output.
- `derived_fact`: symbolic/e-graph facts once rules are attached.
- `hypothesis`: semantic roles and component groups.
- `instruction`: pending reconstruction obligations.
- `validated_instruction`: obligations whose validators pass.
- `accepted_reconstruction`: future canonical merge after review.

## Follow-On Work

- Extract real relocation records from object sections.
- Add Datalog/egglog rules as first-class derived-fact passes instead of the current basic semantic recognizers.
- Replace component recovery heuristics with learned GNN boundary detection when the training stream is available.
- Add richer validators from dynamic traces and golden fixtures.
