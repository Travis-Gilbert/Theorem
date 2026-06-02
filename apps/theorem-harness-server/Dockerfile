# Dockerfile for theorem-harness-server.
#
# Build context must be the Theorem repo root. This app is its own Cargo root,
# but it path-depends on crates under rustyredcore_THG/crates.

FROM rust:1.89-bookworm AS builder
WORKDIR /app

COPY apps/theorem-harness-server ./apps/theorem-harness-server
COPY rustyredcore_THG/crates/theorem-harness-core ./rustyredcore_THG/crates/theorem-harness-core
COPY rustyredcore_THG/crates/theorem-harness-runtime ./rustyredcore_THG/crates/theorem-harness-runtime
COPY rustyredcore_THG/crates/rustyred-thg-core ./rustyredcore_THG/crates/rustyred-thg-core
# Connector driving surface (slice 3) path-deps: the harness-server Cargo.toml
# pulls in rustyred-thg-connectors + rustyred-thg-affordances, so their sources
# must be in the build context too (connectors -> affordances -> core/harness-core,
# the latter two already copied above).
COPY rustyredcore_THG/crates/rustyred-thg-affordances ./rustyredcore_THG/crates/rustyred-thg-affordances
COPY rustyredcore_THG/crates/rustyred-thg-connectors ./rustyredcore_THG/crates/rustyred-thg-connectors

WORKDIR /app/apps/theorem-harness-server
RUN cargo build --release --bin theorem-harness-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/apps/theorem-harness-server/target/release/theorem-harness-server /usr/local/bin/theorem-harness-server

ENV THEOREM_HARNESS_DATA_DIR="/data/theorem-harness"

EXPOSE 8080

CMD ["theorem-harness-server"]
