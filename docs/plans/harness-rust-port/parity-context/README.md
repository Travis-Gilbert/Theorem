# Context-compiler parity corpus (Claude-Code lane)

LANE CLAIM (2026-06-01, substrate down -> git is the channel):

- **claude-code owns this dir** (`docs/plans/harness-rust-port/parity-context/`):
  the Python reference corpus for the context compiler's **pure pack core** (spec
  step 4, the headline property).
- **codex owns** the Rust port (in `theorem-harness-core/**`) and the IO retriever
  (`context_web/retriever.py`, `thg_adapter.py`). This dir is the ORACLE only; the
  Rust pack implementation is Codex's, same split as the kernel.

## What this covers (the pure core, not the IO)

`context_web` splits into a pure decision core (`contracts.py` + `policy.py`, 0
IO) and an IO retriever (1301-line `retriever.py` + `thg_adapter.py` that read the
graph). This corpus covers the pure core:

- **`ContextWebPack.bounded(policy)`** - the capsule-budget enforcement: rank atoms
  by `(-score, id)`, quarantine generated artifacts, enforce `max_atoms` then
  `max_tokens`, filter edges/paths to selected nodes, build the token ledger
  (raw/packed/saved) and the `why_included` / `why_excluded` provenance. This is
  the highest-value harness property and it is pure.
- **`ContextWebBudget.capped_for_mode`** - `mini` mode caps tokens to 300 and atoms
  to 6.
- **`policy.normalize_context_web_node_id` / `is_generated_artifact`** - the path
  normalization + generated-artifact quarantine that `allows_atom` depends on.

Retrieval (getting candidate atoms from the graph) is IO and is NOT here; it is
Tier B (a Rust service over the substrate), sequenced later.

## Files

- `generate_context_fixtures.py` - drives the live Python `ContextWebPack.bounded`
  and the policy functions; records the real output.
- `context_fixtures.json` - the generated corpus (`pack_scenarios` + `policy_cases`).

## Regenerate

```bash
python3 docs/plans/harness-rust-port/parity-context/generate_context_fixtures.py --check
```

Deterministic: `--check` runs twice and asserts byte-identical output. The pack
has no timestamp/hash inputs; ranking is stable by `(-score, id)`.

## Assertion guidance for the Rust port (incremental)

The recorded `expected` is the FULL `ContextWebPack.bounded().to_dict()`. Suggested
order when porting:

- **Phase A (the pack core):** assert `atoms` (selected ids + order), `provenance.why_included` / `why_excluded`, `token_ledger`, `source_mix`, `edges`, `paths`.
- **Phase B (derived summaries):** `validation` and `evaluation` come from
  `_validation_summary` / `_evaluation_summary`; port and assert these second.

## Handoff to Codex

When you port the context pack to Rust (spec step 4), wire a parity test (mirror of
`tests/parity.rs`) against `context_fixtures.json`. The corpus is read-only ground
truth from the live Python reference.
