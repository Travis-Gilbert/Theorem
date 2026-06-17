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
# theorem-harness-runtime path-depends on prose-check (deterministic style checker
# for receipts); its source must be in the build context or cargo resolution fails.
COPY rustyredcore_THG/crates/prose-check ./rustyredcore_THG/crates/prose-check
# Connector driving surface (slice 3) path-deps: the harness-server Cargo.toml
# pulls in rustyred-thg-connectors + rustyred-thg-affordances, so their sources
# must be in the build context too (connectors -> affordances -> core/harness-core,
# the latter two already copied above).
COPY rustyredcore_THG/crates/rustyred-thg-affordances ./rustyredcore_THG/crates/rustyred-thg-affordances
COPY rustyredcore_THG/crates/rustyred-thg-connectors ./rustyredcore_THG/crates/rustyred-thg-connectors
# GitHub App ingestion path-depends on the code graph runtime; include its
# direct local path dependencies too so Railway's cargo build resolves offline
# from the repo-root Docker context.
COPY rustyredcore_THG/crates/rustyred-membrane ./rustyredcore_THG/crates/rustyred-membrane
COPY rustyredcore_THG/crates/rustyred-rerank ./rustyredcore_THG/crates/rustyred-rerank
COPY rustyredcore_THG/crates/rustyred-thg-code ./rustyredcore_THG/crates/rustyred-thg-code
# Dispatch v2 job board: the harness-server Cargo.toml path-depends on
# theorem-dispatch (Postgres execution queue); its source must be in the build
# context too (theorem-dispatch -> theorem-harness-core, already copied above).
COPY rustyredcore_THG/crates/theorem-dispatch ./rustyredcore_THG/crates/theorem-dispatch

WORKDIR /app/apps/theorem-harness-server
# rustyred-thg-affordances pulls in BLAS-backed crates (statrs/nalgebra/faer),
# so the binary links -lopenblas. Install it (and pkg-config) before building.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libopenblas-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --bin theorem-harness-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopenblas-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/apps/theorem-harness-server/target/release/theorem-harness-server /usr/local/bin/theorem-harness-server

ENV THEOREM_HARNESS_DATA_DIR="/data/theorem-harness"

EXPOSE 8080

CMD ["theorem-harness-server"]
