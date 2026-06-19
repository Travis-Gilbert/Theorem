use std::sync::{Arc, Mutex};

use rustyred_thg_core::{InMemoryThgExecutor, ThgExecutor};
use rustyred_thg_pg_server::{
    demo_native_store, serve, serve_relational, SharedExecutor, SharedRelationalStore,
};

fn main() -> std::io::Result<()> {
    let addr =
        std::env::var("RUSTYRED_THG_PG_ADDR").unwrap_or_else(|_| "127.0.0.1:6543".to_string());
    let listener = std::net::TcpListener::bind(&addr)?;
    eprintln!("RUSTYRED_THG_PG_READY {addr}");

    // Default to the spec-faithful planner-backed native-views surface
    // (SPEC-RUSTYRED-PG-WIRE: SQL -> planner QueryIr -> access-method seam).
    // Set RUSTYRED_THG_PG_MODE=executor for the legacy ThgExecutor command
    // surface from lib.rs.
    match std::env::var("RUSTYRED_THG_PG_MODE").as_deref() {
        Ok("executor") => {
            let executor: SharedExecutor = Arc::new(Mutex::new(
                Box::new(InMemoryThgExecutor::new()) as Box<dyn ThgExecutor + Send>,
            ));
            serve(listener, executor)
        }
        _ => {
            // demo_native_store seeds memory + epistemic native views. Swap for a
            // real native-views store wired to the live memory/epistemic relations
            // once that seam lands; auth/billing stay on real Postgres (Neon).
            let store: SharedRelationalStore = Arc::new(Mutex::new(demo_native_store()));
            serve_relational(listener, store)
        }
    }
}
