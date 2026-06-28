# North Star: Theorem as a Compounding Reverse-Engineering Engine

## Intent

Theorem ingests binaries and source, decomposes them into provenance-bearing facts, and lets agents call reconstruction as a tool rather than carry it as a skill. The corpus gets more valuable with every artifact, because standing hypotheses re-fire against new facts and the facts compound. The goal is an automated analyst that improves as you feed it, not a database that you re-interrogate by hand.

This register is intention plus guardrails. The two execution handoffs alongside it carry the enumerated deliverables.

## The shape

One pipeline, five stages, each landing on substrate that already exists in `rustyredcore_THG`:

1. Ingest. A binary or link enters and runs `binformat` then `disasm` then `lift` then `reconstruct`, producing observed, derived, hypothesis, and instruction facts in the versioned graph.
2. Fact corpus. Facts are append-only and seekable. Graph state is a fold over the fact log. Time-travel and streaming fall out of the log being the source of truth.
3. Retrieval. A tiered index plus boolean pushdown lets queries intersect billions of field-level facts in bounded time without the index growing combinatorially.
4. Compute. GraphBLAS sparse-matrix algebra runs graph analytics and interprocedural dataflow as matrix operations over semirings.
5. Standing seeds. Capability hypotheses, shaped like capa rules, register as standing queries that re-evaluate against new facts and emit matches as events.

The agent-facing surface sits on top: tools that ingest, query, register seeds, fetch obligations, and write receipts.

## The five borrowed ideas and where each lives

Each idea comes from a system that solved analysis-at-scale before it was a product category. The pattern is to take the idea and run it on RustyRed, where the facts compound, rather than adopt the system.

- Tiered retrieval (DATAWAVE). A small global index maps value plus field to fragment, and a per-fragment field index resolves the rest above a cardinality threshold. Lands beside `access_method.rs` and the index subsystem, maintained on fragment commit through `hooks.rs`.
- Log-structured standing seeds (LemonGraph). The fact log is the truth, a seed bookmarks its log position, and it re-fires against only the new fragments on commit. Rides `working_log.rs` and `stream.rs`.
- GraphBLAS compute (FalkorDB, SuiteSparse:GraphBLAS). Adjacency as a typed sparse matrix, traversal as matrix-vector multiply over a semiring. Lands beside `graph_csr.rs` and the ML crate.
- Semantic vocabulary (capa). Capabilities recognized from code-level features over instructions, basic blocks, and functions, cross-referenced to MITRE ATT&CK and the Malware Behavior Catalog. Lands in `reconstruct` as derived-fact rules and as labels on epistemic edges.
- Incremental derivation (differential dataflow). Adding a fact updates derived facts and standing seeds incrementally instead of re-deriving. The collection model carries data, time, and a diff. Rides `stream.rs` and the working log.

## The unifying compute insight

GraphBLAS is not only a graph-analytics accelerator. Context-free-language reachability, which underlies interprocedural points-to, taint, and alias analysis, runs as matrix multiplication over GraphBLAS. The same matrix engine therefore powers graph analytics and the dataflow analysis inside the code crawler and the reconstruction engine. This is why sparse matrices belong at the center of the substrate rather than at the edge.

## Guardrails

- The substrate is the upgrade. Hermes keeps memory in markdown files, DATAWAVE assumes Accumulo, FalkorDB ships SSPL. The place Theorem wins is underneath, where facts carry provenance and confidence and compound across artifacts. Borrow loops and designs, not implementations.
- Preserve the authority layers. Every fact carries its layer: observed, derived, hypothesis, instruction, validated, accepted. A role hypothesis is an epistemic claim with provenance and a falsification condition, managed by `epistemic.rs`, not a flat label.
- Agents receive perception, not internals. The tool surface returns obligations, evidence, validators, confidence, and unknowns. Lifter and decoder internals stay behind the capability pack.
- License chain stays commercial-safe. The graph engine binds SuiteSparse:GraphBLAS (Apache-2.0) and LAGraph (BSD) directly. The non-commercial Rust wrapper crates and SSPL FalkorDB code are not used.
- Theorem does the work. Reconstruction is a tool an agent calls with a link, not a procedure an agent performs step by step.

## What done feels like

You hand Theorem a binary and receive a set of reconstruction obligations with evidence and validators. You register a capability hypothesis once, and it re-fires automatically every time a matching binary is ingested. You ask which artifact versions contain a given handler and get an intersection over the index instead of a graph walk. Each binary you reverse raises the value of every prior analysis, because the standing hypotheses now have more to match and the facts now intersect across more of the corpus.

---

## Theorem-side build status (this plan)

The intake half of stages 1-3 lands in `rustyred-thg-datawave` (the DATAWAVE
write-side absorb). See [STATUS.md](STATUS.md) for the phase -> module -> test map
and [DATAWAVE-SCOUT-INGEST-AND-EDGE.md](DATAWAVE-SCOUT-INGEST-AND-EDGE.md) for the
reference. The binary-reconstruction half (stage 1's `binformat`/`disasm`/`lift`/
`reconstruct`) is built in the sibling `rustyred-thg-*` crates and composes over
the same shared graph.
