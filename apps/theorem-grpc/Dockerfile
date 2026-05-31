# Dockerfile for theorem-grpc-server.
#
# BUILD CONTEXT must be the Theorem repo root (Theorem/), NOT apps/theorem-grpc,
# because the crate path-deps reach ../../rustyredcore_THG/crates/{rustyred-web,
# rustyred-thg-core}. On Railway, set the service Root Directory so the Docker
# build context is the Theorem repo root.
#
# Transitive internal-crate set (verified via `cargo tree -p theorem-grpc`):
# exactly rustyred-web + rustyred-thg-core. rustyred-thg-core is a leaf crate
# (no internal path-deps); rustyred-web path-deps only rustyred-thg-core. So the
# two COPYs below are complete.
#
# Toolchain: rust 1.89-bookworm. Above rustyred-thg-server's 1.88, matching the
# civic backend's >=1.89 requirement, and deliberately NOT the Servo 1.95
# toolchain apps/browser implies (no Servo / async-graphql crates are imported
# in slice 1).

FROM rust:1.89-bookworm AS builder
WORKDIR /app

# tonic-build invokes protoc at compile time.
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Context = Theorem repo root. Copy the crate + its path-dep sources.
COPY apps/theorem-grpc ./apps/theorem-grpc
COPY rustyredcore_THG/crates/rustyred-web ./rustyredcore_THG/crates/rustyred-web
COPY rustyredcore_THG/crates/rustyred-thg-core ./rustyredcore_THG/crates/rustyred-thg-core

WORKDIR /app/apps/theorem-grpc
RUN cargo build -p theorem-grpc --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/apps/theorem-grpc/target/release/theorem-grpc-server /usr/local/bin/theorem-grpc-server

# The binary reads PORT from env and binds 0.0.0.0:$PORT (Railway injects PORT).
# EXPOSE is documentation only.
EXPOSE 8080
CMD ["theorem-grpc-server"]
