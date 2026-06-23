#!/usr/bin/env bash
# Run theorem-grpc (code search) locally, with Valkey as the cache backend.
# theorem-grpc is a standalone Cargo root, NOT a rustyredcore_THG member.
#
# Valkey lives HERE, not in the harness server. Valkey speaks the Redis wire
# protocol, so the `redis://` URL connects to a Valkey daemon unchanged. The
# boot ping degrades gracefully: unreachable Valkey logs a warning, does not
# block startup.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${HERE}/theorem-local.env"

# Valkey (Redis-wire compatible). Start a daemon first, e.g.:
#   brew install valkey && valkey-server --port 6379
#   # or: docker run -p 6379:6379 valkey/valkey:latest
export VALKEY_URL="${VALKEY_URL:-redis://127.0.0.1:6379}"
export VALKEY_CACHE_TTL_SECONDS="${VALKEY_CACHE_TTL_SECONDS:-300}"
export VALKEY_KEY_PREFIX="${VALKEY_KEY_PREFIX:-theorem-grpc}"

# PORT must match THEOREM_GRPC_URL in theorem-local.env (default 50071).
export PORT="${PORT:-50071}"

echo "[run-grpc] PORT=${PORT} VALKEY_URL=${VALKEY_URL}"
exec cargo run --manifest-path "${HERE}/../../apps/theorem-grpc/Cargo.toml" -p theorem-grpc
