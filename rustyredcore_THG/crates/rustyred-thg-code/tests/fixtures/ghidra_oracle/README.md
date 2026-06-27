# Ghidra Oracle Fixture

This directory holds the first headless Ghidra oracle lanes for Theorem program
analysis. `ExportTheoremFacts.java` emits the generic `GhidraOracleExport` JSON
envelope consumed by `rustyred-thg-code`. `ExportTheoremSymZ3Facts.java` emits
the same envelope shape with `symbolic_summaries` populated from Ghidra's
SymbolicSummaryZ3 p-code emulator. `ExportTheoremDecompilerDiagnostics.java`
emits decompiler uncertainty and failure receipts into `diagnostics`.
`ExportTheoremBSimSignatures.java` emits Ghidra decompiler/BSim semantic
signature vectors into `semantic_signatures`.
`ExportTheoremFunctionId.java` emits Ghidra FunctionID/FID hash quads into
`function_id_signatures`.

Generic run shape:

```sh
analyzeHeadless <project-dir> theorem-oracle -import hello_tiny.o \
  -postScript ExportTheoremFacts.java hello_tiny.oracle.json
```

For switch/jump-table coverage, use `hello_switch.o` with the same generic
exporter. The generic exporter accepts optional numeric args for max functions
to decompile for jump-table recovery and timeout seconds per function:

```sh
analyzeHeadless <project-dir> theorem-oracle-switch -import hello_switch.o \
  -postScript ExportTheoremFacts.java hello_switch.oracle.json 256 30
```

SymbolicSummaryZ3 run shape:

```sh
analyzeHeadless <project-dir> theorem-symz3-oracle -import hello_tiny.o \
  -postScript ExportTheoremSymZ3Facts.java hello_tiny.symz3.oracle.json 256 128
```

The optional numeric args are max functions and max instructions per function.
The SymZ3 exporter calls `SymZ3.loadZ3Libs()`, copies initialized program memory
into the emulator, executes bounded per-function paths, records branch
preconditions, streams register/memory valuations, and reflects SymZ3's internal
register maps and memory-witness list so load/store witnesses and read/update
register sets are preserved in the oracle envelope.

Decompiler diagnostics run shape:

```sh
analyzeHeadless <project-dir> theorem-decompiler-diagnostics -import hello_tiny.o \
  -postScript ExportTheoremDecompilerDiagnostics.java hello_tiny.diagnostics.oracle.json 256 30
```

The optional numeric args are max functions and timeout seconds per function.
The diagnostics exporter uses `DecompInterface.decompileFunction` and
`DecompileResults` status flags/messages. It does not export decompiled C; it
emits decompiler completion, timeout, cancellation, startup failure, warning,
and error evidence so reconstruction instructions can carry uncertainty and
validators.

BSim/decompiler signature run shape:

```sh
analyzeHeadless <project-dir> theorem-bsim-signatures -import hello_tiny.o \
  -postScript ExportTheoremBSimSignatures.java hello_tiny.bsim.oracle.json 256 30 7
```

The optional numeric args are max functions, timeout seconds per function, and
Ghidra decompiler signature-settings bitmask. The exporter calls
`DecompInterface.generateSignatures` for the unordered BSim control/data-flow
feature hashes and direct call list, then calls `debugSignatures` for optional
human-readable feature descriptions. It does not export decompiled C.

FunctionID/FID hash run shape:

```sh
analyzeHeadless <project-dir> theorem-fid -import hello_tiny.o \
  -postScript ExportTheoremFunctionId.java hello_tiny.fid.oracle.json 256
```

The optional numeric arg is max functions. The exporter calls
`FidService.hashFunction` and records Ghidra's FID hash quad: code-unit size,
full hash, specific additional size, specific hash, the short/medium hash
length constants, and hash algorithm. The initial script emits hash quads even
without an attached FID database; the envelope also supports library match
records for search results.

The export contains:

- `fixture`: Ghidra version, language/compiler ids, source URI, summary counts,
  and evidence ids.
- `functions`: entry point, body start/end, name, and evidence.
- `pcode_ops`: p-code sequence key, mnemonic, opcode id, inputs, and output.
- `references`: from/to address, reference type, operand index, primary flag,
  source type, semantic roles, and memory/register/stack/external flags.
- `call_edges`: source function entry, target function entry, and callsite.
- `jump_tables`: decompiler-recovered switch address, case/default targets,
  label values, display format, load-table address/entry/count facts, override
  state, reference completeness, and evidence.
- `equates`: scalar names, display names, values, display values, inferred
  display formats, optional enum UUIDs, operand-index references, dynamic-hash
  references, and evidence.
- `external_linkages`: external libraries and library paths, parent namespace,
  label and original imported name, Ghidra external-program and EXTERNAL-space
  addresses, source type, function/data-type signatures, local thunk chains,
  and evidence.
- `data_types`: Ghidra data-type manager facts including structs, unions,
  typedefs, pointers, arrays, enums, byte/aligned lengths, explicit
  packing/alignment settings, composite fields, bitfield offsets/sizes, enum
  values, dependencies, and evidence.
- `high_variables`: decompiler high-symbol/high-variable facts including
  variable names, parameter/local/global/return-storage kind, symbol and data
  type ids, name/type locks, `this` and hidden-return flags, first-use
  addresses, variable storage, varnode storage pieces, instances, merge groups,
  and defining p-code ids.
- `symbolic_summaries`: optional SymbolicSummaryZ3-compatible path
  preconditions, register read/update sets, symbolic values, memory witnesses,
  solver status, and model bindings. The generic exporter emits this as an
  empty array; the SymZ3 exporter populates it with `not_checked` solver status
  because it records path/state facts but does not ask Z3 for a model.
- `diagnostics`: optional decompiler diagnostic facts with category, severity,
  placement, message, source pass/rule, affected analysis surfaces, completion
  flags, and evidence. The diagnostics exporter populates this array; the other
  exporters emit it empty.
- `semantic_signatures`: optional BSim/decompiler signature facts with feature
  hashes, debug feature descriptions, direct call targets, signature settings,
  decompiler version, `has_unimplemented`, `has_bad_data`, and evidence. The
  BSim exporter populates this array; the other exporters emit it empty.
- `function_id_signatures`: optional FunctionID/FID hash facts with full and
  specific hashes, code-unit counts, hash configuration, optional library match
  scores, and evidence. The FunctionID exporter populates this array; the other
  exporters emit it empty.

`hello_tiny.oracle.json` is a checked-in fixture matching this envelope plus
loader expectations, one external-linkage import, one data-type layout fact,
one high-variable storage fact, one stack-frame layout fact, and one ParamID
parameter-measure fact used by `tests/program_analysis_oracle.rs`.
`hello_switch.oracle.json` is a checked-in switch-dispatch fixture with one
non-empty `jump_tables` array, one non-empty `equates` array, and matching
`hello_switch.elf.hex` object bytes.

During program-analysis compilation, Ghidra oracle references also derive
`ReferenceRecoveryEvidence` nodes. Those nodes keep native-vs-oracle drift
separate from recovery features while preserving source-reference provenance.
