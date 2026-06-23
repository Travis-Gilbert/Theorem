//! The CommonPlace API HTTP server (plan unit F3 + durable backing).

#[tokio::main]
async fn main() {
    if let Err(error) = commonplace_api::run_from_env().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
