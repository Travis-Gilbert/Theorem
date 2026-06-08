//! Compatibility shim for theorem-grpc transports.
//!
//! Code parsing and code graph operations now live in `rustyred-thg-code` so
//! MCP/HTTP/gRPC adapters can share one tenant-store implementation.

pub use rustyred_thg_code::*;
