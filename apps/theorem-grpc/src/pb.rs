//! Generated-proto include module for theseus_search.v1.
//!
//! `tonic::include_proto!` pulls in the code `tonic-build` emitted from
//! `proto/theseus_search/v1/search.proto` at compile time (see build.rs).
//! Mirrors rustyred-thg-server/src/grpc/mod.rs `tonic::include_proto!("rustyred.v1")`.

pub mod search {
    tonic::include_proto!("theseus_search.v1");
}

// Re-export the trait + server type the service module implements/mounts, plus
// the message types the handlers reference by short name. The trait name is the
// proto service name (`SearchService`); the generated server wrapper is
// `SearchServiceServer`. (Message types only constructed via the `pb::search::`
// path, e.g. ProvenanceEdge/GapClosure, are reachable through `search::*` and
// are not re-exported here to keep the surface to what's actually named.)
pub use search::search_service_server::{SearchService, SearchServiceServer};
pub use search::{
    GapWalkRequest, GapWalkResponse, ProvenanceGraph, ProvenanceNode, ProvenanceRequest,
    SearchRequest, SearchResponse, SearchResult, SourcePairRequest, SourcePairResponse,
};
