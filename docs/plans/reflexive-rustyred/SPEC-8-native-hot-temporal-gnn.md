# SPEC-8: Native HOT, a Higher-Order Temporal GNN for Substrate Memory

Source handoff copied from `/Users/travisgilbert/Downloads/SPEC-8-native-hot-temporal-gnn.md` on 2026-06-21 so future sessions do not depend on a Downloads-only artifact.

Reference sources used during implementation:

- HOT paper: arXiv `2311.18526`.
- DyGLib reference family: `yule-BUAA/DyGLib` commit `3aacc36b94b8d2d8293d70a74fdf6d39089b4163`, especially `models/DyGFormer.py`.
- Block-Recurrent Transformer reference: `lucidrains/block-recurrent-transformer-pytorch` commit `9f28fcdb387d3169724b06d59ceca7024b8cb700`, especially `block_recurrent_transformer_pytorch.py`.

## Register

Execution handoff. Enumerated deliverables, file paths, signatures, observable acceptance, named choices treated as requirements, confirm-points called out.

Source model: HOT, Higher-Order Dynamic Graph Representation Learning with Efficient Transformers (Besta, Catarino, Gianinazzi, Blach, Nyczyk, Niewiadomski, Hoefler; arXiv:2311.18526; Proceedings of Learning on Graphs 2023). Reference implementations: the model extends the DyGLib library and DyGFormer; the hierarchical block is the Block-Recurrent Transformer (Hutchins et al. 2022), with a PyTorch reference at lucidrains/block-recurrent-transformer-pytorch.

## Purpose

Port HOT to native Rust as the temporal learned densification generator in the standing-pass organizer engine (SPEC-7, Section B), paired with the existing Pairformer in `rustyred-thg-adapters`. HOT scores candidate links over the history of graph updates and emits them as advisory densification candidates with support, off the hot path, admitted through the confidence dial. It is the learned organizer for the temporal dimension of substrate memory.

## Architecture Recap

Per candidate pair `(u, v)` at timestamp `t`:

1. Extract higher-order temporal neighbors. Build the 1-hop interaction set for `u`, all `(u, u', t')` with `t' < t`, keep the `s1` most recent. Then the 2-hop set: for each kept `(u, u', t')`, consider `(u', u'', t'')` with `t'' < t` and add `(u, u'', t'')` for the `s2` most recent. Focus on 1-hop and 2-hop. Same for `v`.
2. Construct input feature matrices. For node `u`: node features plus a two-value one-hot marking 1-hop versus 2-hop; edge features; TGAT time-interval encoding for `delta = t - t'`, using the interleaved cosine/sine form with trainable frequencies. Same for `v`.
3. Encode neighbor co-occurrence. For each neighbor `w`, count appearances in the node's own interaction set and in the partner's set; project through two one-hidden-layer ReLU MLPs and sum into the co-occurrence encoding.
4. Patch, align, concatenate. Bundle `P` temporally adjacent rows, align each matrix to dimension `d`, concatenate node/edge/time/co-occurrence horizontally per node, then concatenate `u` and `v` horizontally. HOT's choice is horizontal source/destination concatenation, not vertical.
5. Block-Recurrent Transformer. Use local block attention, cross-attention against recurrent state vectors, state read/write, sigmoid gating, GEGLU feed-forward, extrapolatable position embeddings, sliding-window blocks, and segment caching. Average-pool outputs into node representations.
6. Decoder. A one-hidden-layer ReLU MLP over the pair representation produces a scalar link score with support path.

## Home And Stack

Crate: `rustyredcore_THG/crates/rustyred-thg-adapters`, as a sibling to the Pairformer files (`hot.rs`, `hot_burn.rs`, `hot_cubecl.rs`), mirroring the Pairformer's split of a CPU reference plus a Burn module plus a CubeCL path. Stack: Burn for the model and CubeCL for GPU kernels, the same stack the Pairformer already uses.

## Deliverables

### 1. Bounded Temporal-Subgraph Extractor

A reader over `RedCoreGraphStore`/graph snapshots materializes the bounded higher-order temporal neighborhood for a candidate pair, reusing the existing temporal graph rather than a separate store. The recency caps `s1` and `s2` bound the work.

```rust
pub struct HotTemporalEdge {
    pub source_id: String,
    pub target_id: String,
    pub timestamp: i64,
    pub edge_type: String,
    pub features: Vec<f32>,
}

pub struct HotInput {
    pub nodes: Vec<HotNode>,
    pub temporal_edges: Vec<HotTemporalEdge>,
    pub query_pairs: Vec<(String, String)>,
    pub as_of: i64,
}
```

Acceptance: given a pair and an as-of timestamp, the extractor returns only interactions strictly before the cutoff, capped at `s1` and `s2`, drawn from the live graph.

### 2. Feature Matrix Construction

Build node, edge, and time matrices per node, with the 1-hop/2-hop one-hot appended to node rows and TGAT time-interval encoding for time rows.

Acceptance: the time encoding matches the interleaved cosine/sine form with trainable frequencies; the one-hot distinguishes hops.

### 3. Neighbor Co-Occurrence Encoding

Compute counts of each neighbor within the node's own interaction set and within the partner's set, then project through two one-hidden-layer ReLU MLPs summed into the co-occurrence encoding.

Acceptance: a shared 1-hop neighbor produces a nonzero co-occurrence signal for the pair.

### 4. Patching, Alignment, Horizontal Concatenation

Implement patching, alignment to common dimension `d`, horizontal concatenation of the four matrices per node, and horizontal concatenation of the two nodes' matrices.

Acceptance: concatenation is horizontal, not vertical; patched sequence length is the ceiling of interaction count over `P`.

### 5. Block-Recurrent Transformer In Burn

Implement BRT as a Burn module: vertical/horizontal recurrent cells, multi-head self-attention and cross-attention against recurrent state, sigmoid gating blend, GEGLU feed-forward, extrapolatable position embeddings, sliding-window blocks, and segment caching. The deterministic Rust reference may land first, but the trainable path must exist and learn.

Acceptance: a forward pass over a bounded sequence produces fixed-dimension node representations; attention cost scales linearly in the interaction count for fixed block and patch sizes.

### 6. Link-Prediction Decoder And Output

A one-hidden-layer ReLU MLP over the pair representation produces a scalar link score, wrapped in an output type that mirrors Pairformer.

```rust
pub struct HotLinkScore {
    pub source_id: String,
    pub target_id: String,
    pub score: f32,
    pub support: Option<SupportPath>,
}
```

`SupportPath` is the shared `PairformerSupportPath` shape. HOT populates support from higher-order temporal neighbors.

Acceptance: each query pair yields a score in a calibrated range with an attached support path when support exists.

### 7. Training Pipeline On RunPod

Training objective: dynamic link prediction. Negative sampling schemes: random, historical, inductive. Metrics: Average Precision and AUC. Settings: transductive and inductive.

Default named configuration: aligned encoding dimension 50, time encoding dimension 100, co-occurrence dimension 50, output dimension 172, four attention heads, two BRT cells with the horizontal cell at position one, BRT block size 16, segment size 32, 32 state vectors, Adam learning rate `1e-4`, 50 epochs, batch 100, sequential chronological mini-batch sampling, small `s1` and `s2`.

Acceptance: stage one reaches paper-ballpark AP/AUC on a public continuous-time benchmark before stage two trains on substrate memory data.

Implementation follow-up on 2026-06-21:

- `theorem_training_run hot-train` now trains the native MLP decoder over HOT pair representations from an exported RedCore graph snapshot, writes `hot_model.json`, writes `hot_model_artifact.json`, evaluates held-out AP/AUC/loss, compares deterministic HOT against graph densification, Pairformer densification, and merged reflexive rankers, then writes the evaluated model artifact back to RedCore.
- `theorem_training_run hot-smoke` seeds a timestamped RedCore temporal fixture before running the same training job, so local validation exercises the real export/train/writeback path rather than only an in-memory unit test.
- The primary learning path is `native_mlp_over_hot_pair_representations` until the Burn/CubeCL implementation beats it on held-out AP/AUC plus latency and memory. The Burn module remains the feature-gated trainable model scaffold; the CubeCL patch kernel remains a profiling target, not a default-path dependency.

### 8. Inference As Standing-Pass Generator

A bounded generator scores candidate pairs from cheap heuristics: 2-hop co-occurrence, embedding shortlist, recent-activity pairs. It emits high-confidence `ProposedEdge` advisory candidates with support attached. Admission and confidence ceiling belong to the engine, not HOT.

Acceptance: registered with the organizer engine, HOT runs in the background, emits scored candidates with support, admits through the dial, and never touches a read path.

### 9. Pairformer Pairing And Fusion

HOT and Pairformer emit the same advisory-link-score-with-support shape. Default fusion proposal: score ensemble with support paths unioned, subject to admission-dial confirmation.

Confirm-point: score ensemble versus gate versus separate admitted edge types.

## Named Choices

One-hop and two-hop neighborhoods with capped `s1`/`s2`; Block-Recurrent Transformer; horizontal source/destination concatenation; default training configuration above; bounded-subgraph discipline matching Pairformer; shared advisory-link-score-with-support contract; training on RunPod and native Rust inference.
