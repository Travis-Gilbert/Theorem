//! LoRA adapter catalog over RustyRedCore-THG graph records.
//!
//! The crate stays above `rustyred-thg-core`: it reuses core graph records,
//! stores, and PPR, while keeping adapter-specific routing and fitness logic
//! out of the core executor.

pub mod commands;
pub mod fitness;
pub mod routing;
pub mod types;
pub mod upsert;

pub use commands::{execute_adapter_command, AdapterCommandResponse};
pub use fitness::{
    effective_fitness, find_adapter_by_id, list_adapters, record_fitness, supersede_adapter,
};
pub use routing::{
    adapter_training_centroid, find_adapters_by_query_embedding, find_adapters_for,
    recompute_embedding,
};
pub use types::{
    adapter_node_id, adapter_vector_designation, normalize_tenant_id, object_node_id,
    AdapterFindRequest, AdapterFitnessRecordRequest, AdapterFitnessRecordResult, AdapterGraphStore,
    AdapterListRequest, AdapterRef, AdapterSupersedeResult, AdapterUpsertResult, LoraAdapter,
    DEFAULT_FITNESS_EPSILON, DEFAULT_MIN_FITNESS, DEFAULT_PPR_DAMPING, DEFAULT_PPR_MAX_PUSHES,
    DEFAULT_SHARED_WEIGHT, DEFAULT_THESEUS_HALF_LIFE_DAYS, DERIVED_FROM, FITNESS_SIGNAL,
    LORA_ADAPTER_LABEL, SHARED_WITH, SUPERSEDES, TENANT_LABEL, THG_ADAPTER_SOURCE, TRAINED_ON,
};
pub use upsert::upsert_adapter;

#[cfg(test)]
#[path = "tests/upsert_test.rs"]
mod upsert_test;

#[cfg(test)]
#[path = "tests/routing_test.rs"]
mod routing_test;

#[cfg(test)]
#[path = "tests/fitness_test.rs"]
mod fitness_test;
