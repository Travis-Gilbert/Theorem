#!/usr/bin/env bash
# Local Theorem node: rustyred-thg-server over a RedCore data dir on the external SSD,
# serving memory + coordination at POST /mcp (roadmap Phase A.2). This is what the
# proxy's --memory-url points at (live ambient memory) and what coordination routes to
# (A.4). Pair with valkey-local.sh for the warm tier.
#
# Local-dev only: auth is OFF and it binds localhost. Do not expose this port.
set -euo pipefail

REPO_THG="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../rustyredcore_THG" && pwd)"

# Build deps: openblas (burn/ndarray) on the pkg-config path; target on the SSD so the
# heavy workspace artifacts + dep cache stay off the internal disk.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/SSD Samsung/theorem-recon-target}"
export PKG_CONFIG_PATH="/opt/homebrew/opt/openblas/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
export LIBRARY_PATH="/opt/homebrew/opt/openblas/lib:${LIBRARY_PATH:-}"

# Node config. Embedded mode is the canonical local substrate: RedCore durable
# graph state on the SSD. Redis/Valkey mode is a legacy compatibility path for
# the graph store; use it only when explicitly testing that path.
export RUSTY_RED_HOST="${RUSTY_RED_HOST:-127.0.0.1}"
export RUSTY_RED_PORT="${RUSTY_RED_PORT:-8380}"
export RUSTY_RED_MODE="${RUSTY_RED_MODE:-embedded}"
export RUSTY_RED_DATA_DIR="${RUSTY_RED_DATA_DIR:-/Volumes/SSD Samsung/theorem-local-node}"
export RUSTY_RED_REQUIRE_AUTH="${RUSTY_RED_REQUIRE_AUTH:-false}"
export RUSTY_RED_MCP_ENABLED="${RUSTY_RED_MCP_ENABLED:-true}"

# Real semantic embedder for HippoRAG memory retrieval: auto-detect a running Ollama
# embedding endpoint and point the node at it. Without this, the node falls back to the
# deterministic hash embedder (no relevance). Switching embedders needs a fresh index --
# wipe RUSTY_RED_DATA_DIR (or just the hippo Page/Phrase/Hub nodes) once.
if [ -z "${QWEN3_EMBEDDING_4B_URL:-}" ] && curl -fs http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
  export QWEN3_EMBEDDING_4B_URL="http://127.0.0.1:11434/v1/embeddings"
  export QWEN3_EMBEDDING_4B_MODEL_ID="${QWEN3_EMBEDDING_4B_MODEL_ID:-all-minilm}"
  export QWEN3_EMBEDDING_4B_DIMENSION="${QWEN3_EMBEDDING_4B_DIMENSION:-384}"
  echo "embedder: Ollama ${QWEN3_EMBEDDING_4B_MODEL_ID} (dim ${QWEN3_EMBEDDING_4B_DIMENSION}) at ${QWEN3_EMBEDDING_4B_URL}"
else
  echo "embedder: hash (no Ollama at 127.0.0.1:11434; set QWEN3_EMBEDDING_4B_URL for a real one)"
fi

mkdir -p "$RUSTY_RED_DATA_DIR"
echo "theorem node: ${RUSTY_RED_HOST}:${RUSTY_RED_PORT}  data -> ${RUSTY_RED_DATA_DIR}  (mode=${RUSTY_RED_MODE}, auth=${RUSTY_RED_REQUIRE_AUTH})"
echo "point the proxy at it: --memory-url http://${RUSTY_RED_HOST}:${RUSTY_RED_PORT}/mcp"
# Prefer the prebuilt binary only when explicitly requested. `cargo run` is the safe
# default because it binds launch behavior to the current checkout; the fast path is for
# operators who knowingly want an existing binary from CARGO_TARGET_DIR.
BIN="${CARGO_TARGET_DIR}/debug/rustyred-thg-server"
if [ "${THEOREM_USE_PREBUILT_NODE:-0}" = "1" ] && [ -x "$BIN" ]; then
  exec "$BIN"
fi
cd "$REPO_THG"
exec cargo run -p rustyred-thg-server --bin rustyred-thg-server
