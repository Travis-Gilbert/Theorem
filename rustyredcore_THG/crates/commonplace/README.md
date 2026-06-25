# commonplace

The CommonPlace consumer object model (Item/Collection/Tag/Task) and auto-structuring ingest, graph-native over the RustyRedCore-THG `GraphStore`. This is the personal-database data layer; the substrate provides the persistence.

## Object model

- `Item` (the universal unit), `ItemKind` (`File`/`Note`/`Link`/`Image`/`Doc`/`Task`/`Other`), `ItemBody` (`Empty`/`Inline`/`Blob`), `Residency` (`Local`/`Synced`/`Hosted`), `SourceRef` (`source` plus `external_id`).
- `Collection` (`CollectionKind::{Manual, Auto}`), `Tag`.
- Facade `Commonplace<S: GraphStore, B: BlobStore>`: `put_item`, `put_file`, `get_item`, `items_by_kind`, `item_by_source_ref` (idempotent lookup); collections (`create_collection`, `get_or_create_collection`, `add_to_collection`, `collection_items`); tags; tasks (`add_subtask`, `add_dependency`, `link_about`, `open_tasks`, `tasks_due_between`, `subtask_progress`).
- Blob seam: `BlobStore` (`put`/`get`), `InMemoryBlobStore`, `content_hash` (`sha256:<hex>`). The core `DiskObjectStore` impls `BlobStore` for durable, address-compatible storage.

## Ingest and organize

- `IngestPipeline<E = DeterministicEmbedder>` with the `Embedder` seam: detects note/doc/link/image/task input, embeds it, classifies/creates auto collections from label embeddings, stores blobs, writes a vector-searchable item embedding, files folder metadata, writes `SIMILAR_TO` edges, and resolves near-duplicate entities. Entry points: `ingest`, `ingest_batch` (one snapshot plus one designation), `ingest_routed` (hard-route), `classify_item`, `classify_item_with_source_prior`.
- `organize.rs`: `decide(commonplace, item, policy) -> OrganizeDecision` (`AutoFiled` / `FiledForReview` / `NeedsYou`), `OrganizePolicy`, `NeedsYouReason`, `RoutingRule` plus `route`. Pure cosine, zero model calls.
- Renderable surface: `RenderableObject`, `renderable_from_item`, `OrganizeAction`/`OrganizeActionVerb` (`File`/`Delegate`/`Draft`/`Develop`), `apply_organize_action`.
- Labels/edges: `ITEM_LABEL`, `COLLECTION_LABEL`, `TAG_LABEL`, `ENTITY_LABEL`; `IN_COLLECTION_EDGE`, `HAS_TAG_EDGE`, `SIMILAR_TO_EDGE`, `MENTIONS_ENTITY_EDGE`, the task edges, plus `SOURCE_REF_KEY_PROPERTY`, `ITEM_EMBEDDING_PROPERTY`.

Caveat: re-ingest on the same `SourceRef` reuses the item id (idempotent), but an item stays in its current collection on re-sync because there is no durable edge-delete; re-sync does not re-file.

Path dep: `rustyred-thg-core`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p commonplace
```

Tests: `tests/f1_object_model_acceptance.rs` (includes RedCore restart rehydration), `tests/f2_ingest_acceptance.rs`, `tests/source_intake_acceptance.rs`, plus inline. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
