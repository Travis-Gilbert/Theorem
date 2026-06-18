# Dockerfile for theorem-gateway (browser-facing GraphQL gateway).
#
# BUILD CONTEXT is the Theorem repo root (Theorem/), mirroring theorem-grpc, so
# the deploy shape is uniform across the two Railway services. The gateway
# path-deps into scene-os-core + scene-os-web (the SceneOS add-on) under
# rustyredcore_THG/crates, so the whole crates dir is copied (theorem-grpc's
# anti-COPY-drift pattern). No OpenBLAS: the scene-os crates are serde-only
# leaves and cargo only compiles what the gateway's tree needs.
#
# Toolchain: rust 1.95-bookworm. Matches the local build that ground-truthed
# this crate and is the same image the repo's burn services bumped to, so async-
# graphql / axum / tonic MSRVs are comfortably covered. (Prior Railway breaks in
# this repo came from pinning an image BELOW a dep's MSRV; pin high deliberately.)

FROM rust:1.95-bookworm AS builder
WORKDIR /app

# Build-time system deps:
#   protobuf-compiler: tonic-build invokes protoc to compile the vendored protos.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Context = Theorem repo root. Copy this crate + the whole crates dir (the
# gateway path-deps into scene-os-core + scene-os-web; copying the whole dir
# keeps the build immune to future transitive path-dep drift).
COPY apps/theorem-gateway ./apps/theorem-gateway
COPY rustyredcore_THG/crates ./rustyredcore_THG/crates

WORKDIR /app/apps/theorem-gateway
RUN cargo build -p theorem-gateway --release

FROM debian:bookworm-slim
# Runtime system deps:
#   ca-certificates: outbound TLS to GL-Fusion (and TLS Valkey, if used).
# No OpenBLAS, no git: the gateway does no numeric linking and never clones
# (the code service clones server-side; the gateway only forwards the URL).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/apps/theorem-gateway/target/release/theorem-gateway /usr/local/bin/theorem-gateway

# The binary reads PORT from env and binds 0.0.0.0:$PORT (public HTTP surface).
# EXPOSE is documentation only.
EXPOSE 8080
CMD ["theorem-gateway"]
