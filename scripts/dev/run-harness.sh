#!/usr/bin/env bash
# Run the harness MCP server (rustyred-thg-server) locally with the gRPC,
# embeddings, search, and browser surfaces wired from theorem-local.env.
#
# All four surfaces degrade gracefully if their satellite isn't running, so this
# boots fine on its own; start the satellites you actually want to exercise:
#   - gRPC code search : ./run-grpc.sh   (+ a Valkey daemon)
#   - embeddings       : an OpenAI-compatible embedding server on :8081
#   - search           : a SearXNG instance on :8888
#   - browser          : optional render / live-action sidecars (see README)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${HERE}/theorem-local.env"

mkdir -p "${RUSTY_RED_DATA_DIR}"
echo "[run-harness] http://${RUSTY_RED_HOST}:${RUSTY_RED_PORT}  data=${RUSTY_RED_DATA_DIR}"
echo "[run-harness] gRPC=${THEOREM_GRPC_URL:-<off>}  embed=${RUSTYWEB_QWEN4B_EMBED_URL:-<off>}  search=${RUSTYWEB_SEARCH_PROVIDERS:-<off>}"
exec cargo run --manifest-path "${HERE}/../../rustyredcore_THG/Cargo.toml" -p rustyred-thg-server --bin rustyred-thg-server
