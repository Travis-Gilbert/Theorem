# reconstruction-engine (Theorem port)

The procedural reconstruction engine, ported **whole** from
`our-civic-atlas-backend/crates/civic-atlas-reconstruction-engine` into Theorem
for the substrate-native browser.

It is the **generative projection class** for SceneOS: where placement
projections (geo, force-graph, matrix) arrange atoms that already exist, this
engine *generates* atoms procedurally from structured facts + a learned prior —
the capability that lets Theseus show a thing it has never seen a picture of,
built from what it knows about the thing.

## Status

- **Compiles, 9 tests pass** (the full 8-stage pipeline, the generic
  `run_domain_pipeline`, the building reference impl, merge-conflict logic).
- Step 1 of the browser plan (generalize the engine) is **already done**: the
  `ReconstructionDomain` trait + generic `run_domain_pipeline` + `PriorModel` /
  `AssetGenerator` port seams are all here, with `BuildingDomain` as the
  reference implementation.

## Temporary coupling (the deliberate part)

Ported whole, building primitives included — they aren't hurting anything, and a
clean strip is worth its own focused pass rather than a rushed genericization.
So this crate currently **path-deps `civic-atlas-types` + `theseus-client`** for:

- the building protobuf types (`Mass`, `Facade`, `Roof`, `ReconstructionSpec`, …),
- the civic Postgres persistence layer (`sqlx`),
- the Pairformer prior's substrate bridge (`theseus-client` gRPC).

It is a **standalone workspace, intentionally NOT in `rustyredcore_THG`'s
members**, so the main Theorem build + CI do not build it and are not coupled to
the civic-atlas repo. Build it directly: `cargo test` in this directory (requires
`our-civic-atlas-backend` present as a sibling).

## Planned strip session (make it self-standing + wire it)

1. Vendor/genericize the building types off the core (`<S = ReconstructionSpec>`
   → a neutral spec; the `EvidenceBundle`/`Artifact` structs that name
   `CivicObject`/`ReconstructionSource`/`TenantContext` concretely → generic).
2. Replace the Postgres persistence with Theorem's substrate
   (`rustyredcore_THG`), drop `sqlx`.
3. Replace the Pairformer civic prior with a prior over Theorem's substrate,
   drop `theseus-client`.
4. Drop the civic path-deps; add the crate to `rustyredcore_THG` members.
5. Add a browser-side `ReconstructionDomain` (mechanism / patent / process) and
   wire `AssetGenerator` output into the SceneOS atom substrate (browser plan
   step 2).

Buildings remain civic-atlas's reference `ReconstructionDomain`; the Theorem copy
diverges to serve browser domains. The two are meant to do different things.
