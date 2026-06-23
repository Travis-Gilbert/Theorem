//! The CommonPlace API HTTP server (plan unit F3 + durable backing).
//!
//! Serves the consumer GraphQL profile over one instance. A client connects with
//! this instance URL plus a key (the `x-api-key` header). `POST /graphql` runs
//! operations, `GET /graphql` serves GraphiQL, `GET /healthz` is liveness.
//!
//! The seed API key comes from `COMMONPLACE_API_KEY` (default `dev-key` for
//! local use); the bind port from `PORT` (default 50090). Set
//! `COMMONPLACE_DATA_DIR` to persist durably (RedCore + disk under that dir);
//! unset = an ephemeral in-memory instance. The router + handlers live in
//! `commonplace_api::serve` so the binary, the acceptance tests, and an
//! in-process embedder (the desktop shell) all share one surface.

#[tokio::main]
async fn main() {
    if let Err(error) = commonplace_api::run_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
