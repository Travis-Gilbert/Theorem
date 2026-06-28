# rustyred-thg-offload

Native compute-offload seed for Theorem.

This crate treats LLM inference as the expensive operator. It provides:

- an operation algebra and cost-based planner,
- predicate-pushdown and model-operation fusion traces,
- a version-keyed computation cache that rejects stale graph results,
- an isotonic-calibrated model cascade,
- CPU graph affordance receipts, and
- verification-offload receipts over graph evidence.

The crate is intentionally portable at the planner layer. Theorem-specific graph
execution stays behind the graph affordance adapter.
