# Dockerfile for theorem-grpc-server.
#
# BUILD CONTEXT must be the Theorem repo root (Theorem/), NOT apps/theorem-grpc,
# because the crate path-deps reach sibling crates under
# ../../rustyredcore_THG/crates. On Railway, set the service Root Directory so
# the Docker build context is the Theorem repo root.
#
# Transitive internal-crate set (verified via `cargo tree -p theorem-grpc`):
# rustyred-web, rustyred-thg-core, rustyred-thg-code, rustyred-thg-affordances,
# theorem-harness-core, pilot-core.
# rustyred-thg-core and theorem-harness-core are leaf crates (no internal
# path-deps); rustyred-web path-deps rustyred-thg-core + pilot-core;
# rustyred-thg-code path-deps rustyred-thg-core; rustyred-thg-affordances
# path-deps rustyred-thg-core + theorem-harness-core. So the COPY list below is
# complete. This list MUST be kept in sync with apps/theorem-grpc/Cargo.toml.
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

# Context = Theorem repo root. Copy the crate + its path-dep sources.
COPY apps/theorem-grpc ./apps/theorem-grpc
COPY rustyredcore_THG/crates/rustyred-web ./rustyredcore_THG/crates/rustyred-web
COPY rustyredcore_THG/crates/pilot-core ./rustyredcore_THG/crates/pilot-core
COPY rustyredcore_THG/crates/rustyred-thg-core ./rustyredcore_THG/crates/rustyred-thg-core
COPY rustyredcore_THG/crates/rustyred-thg-code ./rustyredcore_THG/crates/rustyred-thg-code
COPY rustyredcore_THG/crates/rustyred-thg-affordances ./rustyredcore_THG/crates/rustyred-thg-affordances
COPY rustyredcore_THG/crates/theorem-harness-core ./rustyredcore_THG/crates/theorem-harness-core

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
