# Standalone Railway-template Dockerfile for the THG variant of
# RustyRed. Produces a `rusty-red-graph-server` binary (matches the
# Railway service name) by building the renamed Cargo crate
# `rustyred-thg-server` (was `thg-product-server` before the C1 cutover
# at commit 26282991) and copy-renaming the output.

FROM rust:1.88-bookworm AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libopenblas-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
COPY vendor ./vendor

RUN cargo build -p rustyred-thg-server --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopenblas-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy-rename: source binary `rustyred-thg-server` → runtime path
# `/usr/local/bin/rusty-red-graph-server` so the existing CMD and any
# operator habits that expect that binary name keep working.
COPY --from=builder /app/target/release/rustyred-thg-server /usr/local/bin/rusty-red-graph-server

ENV RUSTY_RED_HOST="[::]" \
    RUSTY_RED_MODE="embedded" \
    RUSTY_RED_DURABILITY="aof_everysec" \
    RUSTY_RED_SNAPSHOT_INTERVAL_WRITES="1000" \
    RUSTY_RED_REQUIRE_VOLUME=true \
    RUSTY_RED_REQUIRE_AUTH=false \
    RUSTY_RED_KEY_PREFIX="rusty-red:tenant" \
    RUSTY_RED_SERVICE_NAME="rusty-red-graph-database" \
    RUSTY_RED_API_TITLE="Rusty Red Graph Database API" \
    RUSTY_RED_MCP_ENABLED=true \
    RUSTY_RED_MCP_READ_ONLY=true \
    RUSTY_RED_MCP_ALLOW_ADMIN=false \
    THEOREM_APP_AFFORDANCE_GRPC_URL="http://theorem-grpc.railway.internal:8080"

EXPOSE 8380

CMD ["rusty-red-graph-server"]
