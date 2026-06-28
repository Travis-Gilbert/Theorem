#!/usr/bin/env bash
# Local Valkey warm-cache tier for the Theorem node (roadmap Phase A.1).
# Data + AOF live on the external SSD; Valkey is Redis-wire-compatible, so
# RedisGraphStore connects to it as the warm/cold tier. Run in the foreground;
# background it (or wrap in launchd) for a persistent local stack.
# Prefers valkey-server; falls back to redis-server (same wire protocol).
set -euo pipefail

DATA_DIR="${THEOREM_VALKEY_DIR:-/Volumes/SSD Samsung/theorem-valkey}"
PORT="${THEOREM_VALKEY_PORT:-6391}"

if command -v valkey-server >/dev/null 2>&1; then
  SERVER=valkey-server
elif command -v redis-server >/dev/null 2>&1; then
  SERVER=redis-server
else
  echo "neither valkey-server nor redis-server on PATH; brew install valkey" >&2
  exit 1
fi

mkdir -p "$DATA_DIR"
echo "$SERVER: data + AOF -> $DATA_DIR, port $PORT"
exec "$SERVER" \
  --dir "$DATA_DIR" \
  --port "$PORT" \
  --appendonly yes \
  --appendfsync everysec \
  --save ""
