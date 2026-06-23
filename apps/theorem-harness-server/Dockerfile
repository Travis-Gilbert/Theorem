# Dockerfile for theorem-harness-server.
#
# Build context must be the Theorem repo root. This app is its own Cargo root,
# but it path-depends on crates under rustyredcore_THG/crates.

FROM rust:1.89-bookworm AS builder
WORKDIR /app

COPY apps/theorem-harness-server ./apps/theorem-harness-server
# Copy the whole crates directory wholesale instead of enumerating each crate.
# theorem-harness-server path-depends on a growing set under
# rustyredcore_THG/crates (harness-core/runtime, thg-core, prose-check,
# affordances, connectors, membrane, rerank, thg-code, theorem-dispatch, ...).
# A per-crate COPY list silently drifts: the moment a new path-dep lands without
# a matching COPY line, the Railway build dies with exit 101 ("failed to load
# manifest for dependency ..."). A whole-dir COPY cannot drift. Matches the
# theorem-grpc Dockerfile (commit 622d202e), which fixed the identical failure.
COPY rustyredcore_THG/crates ./rustyredcore_THG/crates

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
