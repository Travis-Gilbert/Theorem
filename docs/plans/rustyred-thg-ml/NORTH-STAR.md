# RustyRed THG ML Runtime And Morphological Graph North Star

Status: implementation started. Shared message-passing and multi-vector
retrieval primitives are landing in `rustyred-thg-ml`.

Source inputs:

- `/Users/travisgilbert/Downloads/RUSTYRED-THG-ML-RUNTIME-NORTHSTAR.md`
- `RustyRed-Graph-Database` commit `fbef2a6644e255c1a6019da9ce224619bff1d06e`, `docs/plans/morphological-graph/BURN-MORPHOLOGICAL-GRAPH-NORTHSTAR.md`
- Live THG code in `rustyredcore_THG` on 2026-06-23

## Decision

The Burn morphological-graph implementation target moves to THG.

Author the reusable ML primitive in the Theorem workspace as a new
`rustyred-thg-ml` crate, then let the standalone `RustyRed-Graph-Database`
receive the capability downstream like the rest of the public RustyRed surface.
Do not build the message-passing primitive first in the standalone repo and
then depend on it from THG; that inverts the current source-of-truth direction
and splits the primitive from the reranker, reflexive, affordance, and harness
consumers that need it first.

The morphological Burn plan is not a separate side quest. It is the first
oracle-checked consumer of the shared THG ML runtime.

## Current THG Reality

The downloaded ML runtime north star correctly points at the THG line, but one
fact has already moved: Burn is no longer absent from THG. Current `main` has
Burn and CubeCL behind `rustyred-thg-adapters` feature
`pairformer-burn-cubecl`; `rustyred-thg-server` enables that feature.

Existing seams:

- `rustyred-membrane` owns `Candidate`, `ScoreContext`, and the `Scorer` trait.
- `rustyred-rerank` owns `RerankScorer`, `ArmWeights`, `CrossEncoder`,
  `ListwiseReranker`, and `BenchmarkLedger`. Today it is fixed-weight plus
  lexical/HTTP rerankers.
- `rustyred-hipporag` generates graph candidates that flow into the membrane.
- `rustyred-web` already gates unified fresh/warm pools through the membrane
  and stamps reranker versions into receipts.
- `rustyred-thg-adapters` already contains the specialized learned/reflexive
  organ: deterministic Pairformer, trainable Burn Pairformer, HOT, EdgeMPNN,
  a `MessageAggregator` trait, fixed-point aggregation, Burn scatter-add, and
  CubeCL launch paths.
- `rustyred-thg-geotemporal` is the current THG crate for tenant-scoped
  geotemporal indexing over core H3/S2 spatial support. The standalone
  "geometry plugin" name does not directly map into THG.

The plan is therefore extraction and unification, not greenfield ML.

## New Crate: `rustyred-thg-ml`

`rustyred-thg-ml` is the THG-owned reusable ML primitive crate.

Default build:

- Depends on `rustyred-thg-core`, `serde`, and small utility crates only.
- Has no default Burn, CubeCL, Candle, or server dependency.
- Exposes deterministic CPU/reference implementations as parity oracles.

Feature-gated tensor build:

- `burn` feature enables Burn tensor modules and CPU/WGPU backends.
- `cubecl` feature enables CubeCL kernel launch paths where they beat Burn's
  portable scatter.
- The feature gate follows the existing adapter rule: tensor backends are
  integration dependencies, never graph-storage dependencies.

Core types:

```rust
pub struct GraphTensorBatch {
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub edge_src: Vec<usize>,
    pub edge_dst: Vec<usize>,
    pub edge_type: Vec<usize>,
    pub edge_confidence: Vec<f32>,
    pub node_features: Vec<Vec<f32>>,
    pub edge_features: Vec<Vec<f32>>,
    pub relation_types: Vec<String>,
}

pub trait MessageAggregator {
    fn aggregate(
        &self,
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        mean_aggregate: bool,
    ) -> ThgResult<Vec<Vec<f32>>>;

    fn aggregator_id(&self) -> &'static str;
}
```

Initial modules:

- `batch`: lower `GraphSnapshot` / bounded graph neighborhoods into
  `GraphTensorBatch` without choosing a model.
- `message_passing`: gather/select source rows, compose messages, scatter-add
  to destination rows, and mean/sum aggregate.
- `reference`: deterministic fixed-point aggregation, used as the oracle for
  Burn/CubeCL.
- `burn`: Burn tensor implementation of the same message-passing contract.
- `multivector`: exact MaxSim, sign-bit binary projection, Hamming MaxSim
  candidate scoring, ranking helpers, and cold/exact-vector manifest shapes.
- `artifacts`: model artifact descriptors and weight-pointer helpers shared by
  rerank, morphology, Pairformer, HOT, and affordance policy.

The crate does not mutate topology. It returns representations, scores, or
advisory candidates. Admission and graph mutation stay in the existing
membrane, standing-pass, quarantine, and store layers.

## Multi-Vector Retrieval Spine

The ColPali/Vespa direction starts in `rustyred-thg-ml` as a scoring and storage
contract before adding an inference runtime.

Implemented first:

- `MultiVectorEmbeddingSet`: exact float vectors plus content/model identity.
- `MultiVectorManifest`: cold exact-vector reference, binary projection
  reference, vector count, dimension, and byte accounting.
- `exact_maxsim_score` / `rank_exact_maxsim`: the CPU oracle for ColPali-style
  late interaction.
- `quantize_sign_bits`: dependency-light binary projection for candidate
  generation.
- `binary_hamming_maxsim_score` / `rank_binary_hamming_maxsim`: Vespa-inspired
  binary/Hamming MaxSim candidate scoring.
- `recall_against_exact_top_k`: measures binary candidate overlap against the
  exact float oracle.
- `rerank_exact_maxsim_bounded`: hydrates exact vectors only for a bounded
  candidate budget, then reranks with exact MaxSim.
- `storage_costs`: reports exact `f32`, exact `f16`, and binary projection byte
  costs for capacity planning.

Current boundary:

- Exact float vectors are treated as cold payloads or bounded test fixtures, not
  node properties.
- Binary projections are the hot candidate-generation shape.
- Candle/ColPali inference remains a later feature-gated producer of
  `MultiVectorEmbeddingSet`, after the exact and binary scorers have stable
  fixtures.

Acceptance:

- Exact MaxSim ranks a region-matching fixture above a weak document.
- Binary Hamming MaxSim matches the exact top-1 on a sign-stable fixture.
- Manifest byte accounting shows the binary projection is materially smaller
  than exact `f32` storage.
- Recall reports quantify binary candidate overlap against exact top-k.
- Bounded rerank proves exact vector hydration is limited to the requested
  candidate budget.
- Dimension mismatches fail with structured THG errors.

## Extraction Path From Existing Code

Start by moving the reusable parts, not the whole learned organ:

1. Move or mirror the `MessageAggregator` trait from
   `rustyred-thg-adapters::edge_mpnn` into `rustyred-thg-ml`.
2. Move or mirror fixed-point aggregation and scatter path selection from
   `rustyred-thg-adapters::reflexive`.
3. Move or mirror `aggregate_messages_burn` and `BurnAggregator` from
   `rustyred-thg-adapters::burn_mpnn`.
4. Leave Pairformer, HOT, reflexive executor, quarantine, and standing-pass
   generator policy in `rustyred-thg-adapters`; make that crate consume
   `rustyred-thg-ml` for tensor/message-passing primitives.

This makes the current specialized work the proving ground while pulling the
general primitive into the crate where morphology and rerank can share it.

## Consumer One: Morphological GNN

The standalone Burn morphological North Star becomes a THG plan slice:

- Geometry/spatial algorithms land through THG reality, not standalone plugin
  names: `rustyred-thg-geotemporal` plus `rustyred-thg-core` spatial support
  for H3/S2, and a new morphology module/crate only if geotemporal becomes too
  broad.
- The Python city2graph lane remains the oracle and active production source
  until parity is measured.
- Port only the spine: enclosed morphological tessellation, rook/queen
  contiguity, two-cap reachability, dual-graph street topology, and typed
  graph lowering.
- Decline city2graph academic surface with no THG consumer: GTFS, mobility flow
  graphs, and metapath construction.

Typed output contract:

- `("place", "touched_to", "place")`
- `("movement", "connected_to", "movement")`
- `("place", "faced_to", "movement")`

The GNN side consumes the same `GraphTensorBatch` and message-passing primitive
as other THG learned organs. Geometry builds the typed graph; `rustyred-thg-ml`
runs message passing over it; admission/wiring waits until Python oracle parity
is proven on the same Flint fixtures.

## Consumer Two: Rerank

Rerank upgrades in two arcs.

Arc zero needs no Burn:

- Learn `ArmWeights` from existing membrane/search/context receipts.
- Keep `RerankScorer` and the `Scorer` seam unchanged.
- Compare fixed weights versus learned weights through `BenchmarkLedger` and
  existing ordering-quality fixtures.

Arc one uses `rustyred-thg-ml`:

- Add a graph-message-passing scorer arm over the bounded candidate/context
  subgraph: query/task seed, active nodes, candidates, provenance edges,
  context-use receipts, support/contradiction/tension edges, and source
  reliability.
- Feed its score into `RerankScorer` as a graph-learned arm rather than
  replacing lexical relevance or PPR.
- Keep the CrossEncoder path independent: an in-process Burn sequence
  classifier can later implement `CrossEncoder`, but text encoding is not the
  same primitive as graph message passing.

The shared primitive is the graph-side scorer, not the external HTTP reranker
replacement. That distinction keeps the runtime honest.

## Consumer Three: Existing Reflexive And Affordance Learning

`rustyred-thg-adapters` should become the first internal consumer of
`rustyred-thg-ml`, not a competing home for the primitive.

Follow-on consumers:

- EdgeMPNN global completion uses `rustyred-thg-ml::MessageAggregator`.
- Pairformer and HOT keep their model-specific blocks but share batch/artifact
  conventions.
- `rustyred-thg-affordances` can later replace the current PPR+fitness warm
  selector with a learned policy head over the same graph batch shape.

The invariant from Reflexive RustyRed remains binding: learned code ranks or
steers within bounded enumerated spaces and never authors free-form graph
mutations.

## Execution Arcs

### Arc 0: Plan And Baseline

- Land this plan as the durable THG target.
- Preserve the standalone morphological Burn doc as a source reference by
  commit hash, not as the implementation location.
- Confirm the exact Python city2graph oracle fixtures and export shape before
  writing morphology code.

Acceptance:

- A future implementer can start from this file and know that THG is the
  authoring home.
- No standalone repo dependency is introduced.

### Arc 1: Shared Primitive Crate

- Add `rustyred-thg-ml` to the workspace.
- Implement `GraphTensorBatch`, deterministic aggregation, and the Burn scatter
  path behind an optional feature.
- Move adapters to depend on it without changing observable Pairformer/EdgeMPNN
  results.

Acceptance:

- `cargo test --manifest-path rustyredcore_THG/Cargo.toml -p rustyred-thg-ml`
  passes.
- Existing adapter tests pass with default features.
- Feature-gated Burn aggregation matches the fixed-point oracle on a small
  graph batch.

### Arc 2: Rerank First Win

- Learn `ArmWeights` from recorded retrieval/context outcomes.
- Benchmark through `BenchmarkLedger`.
- Keep `Scorer` and membrane API stable.

Acceptance:

- Learned weights beat or tie fixed weights on the held fixture set.
- A worse learned fit is rejected and fixed weights remain default.

### Arc 3: Morphological Geometry Contract

- Build the THG morphology typed-graph contract and fixture exporter.
- Materialize city2graph oracle outputs for Flint blocks.
- Implement dual-graph street topology first because `connected_to` is the
  smallest parity surface and was the first standalone handoff target.

Acceptance:

- Same inputs produce comparable `connected_to` edge sets against the Python
  oracle with stated tolerance.
- No GNN is needed yet; this proves the geometry spine.

### Arc 4: Morphological Message Passing

- Lower typed morphology graph outputs to `GraphTensorBatch`.
- Add the first morphology GNN head over `rustyred-thg-ml` message passing.
- Keep output advisory until edge-set parity and model metrics are both proven.

Acceptance:

- Tensor lowering preserves node ids, relation labels, edge ids, and support.
- The GNN path can run on CPU and optional Burn backend with deterministic
  parity fixtures.

### Arc 5: Graph-Aware Rerank Arm

- Build a bounded candidate subgraph for rerank.
- Run the same message-passing primitive to produce a graph-learned score.
- Compare against fixed PPR-only graph term through the existing web/code
  retrieval acceptance fixtures.

Acceptance:

- Graph-learned arm improves or ties ordering quality without raising token
  admission cost.
- Reranker version receipts identify the learned arm and weight set.

### Arc 6: Downstream Standalone Flow

- Once THG proves the crate and consumers, flow the public subset into
  `RustyRed-Graph-Database`.
- The standalone plan becomes the downstream packaging target, not the
  implementation authority.

Acceptance:

- Public repo receives the shared primitive without forking model semantics.
- THG remains the place where harness, rerank, affordance, and morphology
  signals compound.

## Validation Discipline

Every arc needs an oracle:

- Primitive: fixed-point aggregation is the byte/number oracle for Burn/CubeCL.
- Morphology: Python city2graph edge sets are the oracle.
- Rerank: `BenchmarkLedger`, ordering fixtures, and membrane receipts are the
  oracle.
- Affordances/reflexive: advisory candidates must preserve support paths and
  never mutate topology directly.

Do not claim replacement of the Python morphology lane until the same inputs
produce edge parity. Do not claim rerank improvement without a benchmark row.
Do not put tensor fields on `NodeRecord` or `EdgeRecord`; use model artifacts,
sidecars, and receipts.

## Open Confirm-Points

- Does morphology live inside `rustyred-thg-geotemporal`, or does it deserve
  `rustyred-thg-morphology` once tessellation/reachability grows beyond index
  composition?
- What is the canonical fixture package for Flint blocks: checked-in small
  fixtures, generated fixtures from Theseus, or both?
- Should `rustyred-thg-ml` own model artifact descriptors, or should those stay
  in `rustyred-thg-adapters::training_substrate` until the second consumer lands?
- Which Rust toolchain is the feature-gated Burn path allowed to require, given
  `rustyred-thg-adapters` currently notes Burn 0.21 requires Rust 1.92+ while
  the workspace minimum is 1.85?

## Non-Goals

- Not a training framework. Heavy training stays external; THG serves and
  adapts bounded models.
- Not a model zoo. Models must consume resident graph structure or existing
  THG outcome signals.
- Not a new mutation path. Learned outputs are representations, scores, or
  advisory candidates until admitted by existing THG mechanisms.
- Not a standalone-first port. Standalone RustyRed receives this downstream
  after the THG runtime proves it.
