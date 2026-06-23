# Dockerfile for commonplace-api (the CommonPlace consumer GraphQL server, plan unit F3).
#
# BUILD CONTEXT must be the Theorem repo root (Theorem/), NOT apps/commonplace-api,
# because the crate path-deps reach sibling crates under
# ../../rustyredcore_THG/crates. On Railway, leave the service Root Directory at
# the repo root so the Docker build context is the Theorem repo root.
#
# The crate's path-dep closure is exactly {commonplace, rustyred-thg-core}:
# rustyred-thg-core has no intra-repo path-deps, and commonplace depends only on
# it, so just those two crate dirs are copied. No system-linked native deps are
# in the closure (no BLAS / OpenSSL-sys / protoc), so the build needs no apt
# packages beyond ca-certificates at runtime.
#
# Toolchain: rust 1.95-bookworm (matches theorem-gateway; async-graphql 7 plus
# the full rustyred-thg-core, including egg, build clean on it).

FROM rust:1.95-bookworm AS builder
WORKDIR /app

# Context = Theorem repo root. Copy the app crate + its path-dep crates.
COPY apps/commonplace-api ./apps/commonplace-api
COPY rustyredcore_THG/crates/commonplace ./rustyredcore_THG/crates/commonplace
COPY rustyredcore_THG/crates/rustyred-thg-core ./rustyredcore_THG/crates/rustyred-thg-core

WORKDIR /app/apps/commonplace-api
RUN cargo build -p commonplace-api --release --bin commonplace-api

FROM debian:bookworm-slim
# Runtime: ca-certificates for outbound TLS (rustls roots).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/apps/commonplace-api/target/release/commonplace-api /usr/local/bin/commonplace-api

# Reads PORT from env, binds [::]:$PORT (IPv6 dual-stack so Railway's IPv6
# private network + healthcheck reach it; also accepts IPv4). EXPOSE is docs only.
EXPOSE 8080
CMD ["commonplace-api"]
