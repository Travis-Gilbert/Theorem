//! Generated-proto include module for the gateway's gRPC clients.
//!
//! `tonic::include_proto!` pulls in the code `tonic-build` emitted from the
//! vendored protos at compile time (see build.rs). Mirrors
//! theorem-grpc/src/pb.rs, but exposes the CLIENT types (this crate dials
//! theorem-grpc) rather than the server types.

pub mod search {
    tonic::include_proto!("theseus_search.v1");
}

// The generated `IngestEvent` oneof has a large-variant spread (the `Finished`
// variant embeds the full `IngestCodebaseResponse`). That is prost-generated
// code we never hand-edit, so the lint allow is scoped to this module only.
#[allow(clippy::large_enum_variant)]
pub mod code {
    tonic::include_proto!("theorem_code.v1");
}

// Re-export the generated gRPC clients the GatewayContext constructs and holds.
pub use code::code_crawler_service_client::CodeCrawlerServiceClient;
pub use search::search_service_client::SearchServiceClient;
