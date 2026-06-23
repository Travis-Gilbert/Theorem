# Dockerfile for theorem-grpc-server.
#
# BUILD CONTEXT must be the Theorem repo root (Theorem/), NOT apps/theorem-grpc,
# because the crate path-deps reach sibling crates under
# ../../rustyredcore_THG/crates. On Railway, set the service Root Directory so
# the Docker build context is the Theorem repo root.
#
# The whole crates directory is copied wholesale (COPY rustyredcore_THG/crates)
# rather than a hand-listed subset. cargo only compiles what theorem-grpc's
# dependency tree needs, so the extra sources cost no build time, and the build
# can never again break when a transitive path-dep is added to a copied crate
# without updating this file. (Previously the per-crate COPY list drifted when
# rustyred-web gained rustyred-membrane + rustyred-rerank, breaking the build
# with cargo exit 101: failed to load source for a missing crate directory.)
#
# Toolchain: rust 1.89-bookworm. Above rustyred-thg-server's 1.88, matching the
# civic backend's >=1.89 requirement, and deliberately NOT the Servo 1.95
# toolchain apps/browser implies (no Servo / async-graphql crates are imported
# in slice 1).

FROM rust:1.89-bookworm AS builder
WORKDIR /app

# Build-time system deps:
#   protobuf-compiler: tonic-build invokes protoc at compile time.
#   libopenblas-dev + gfortran: rustyred-web pulls nalgebra/ndarray/statrs/cblas
#     which link OpenBLAS (-lopenblas) at the final link step.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        protobuf-compiler \
        libopenblas-dev \
        gfortran \
    && rm -rf /var/lib/apt/lists/*

# Context = Theorem repo root. Copy the app crate + the whole crates dir, so the
# build is immune to path-dep drift.
COPY apps/theorem-grpc ./apps/theorem-grpc
COPY rustyredcore_THG/crates ./rustyredcore_THG/crates

WORKDIR /app/apps/theorem-grpc
RUN cargo build -p theorem-grpc --release

FROM debian:bookworm-slim
# Runtime system deps:
#   ca-certificates: outbound TLS.
#   libopenblas0: the OpenBLAS shared library the binary dynamically links.
#   git: code_search.ingest can shallow-clone repo URLs at runtime.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        libopenblas0 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/apps/theorem-grpc/target/release/theorem-grpc-server /usr/local/bin/theorem-grpc-server

# The binary reads PORT from env and binds [::]:$PORT (IPv6 dual-stack, so
# Railway's IPv6 private network reaches it). EXPOSE is documentation only.
EXPOSE 8080
CMD ["theorem-grpc-server"]
