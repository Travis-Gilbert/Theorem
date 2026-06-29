#!/usr/bin/env bash
# Local Valkey warm-cache tier for the Theorem node (roadmap Phase A.1).
# Data + AOF live on the external SSD; Valkey is Redis-wire-compatible, so
# RedisGraphStore can connect to it when `RUSTY_RED_MODE=redis`; embedded-mode
# RedCore remains the canonical local graph substrate. Run in the foreground by
# default, or set THEOREM_VALKEY_DAEMONIZE=true for a persistent local daemon.
# Prefers valkey-server; falls back to redis-server (same wire protocol).
set -euo pipefail

DATA_DIR="${THEOREM_VALKEY_DIR:-/Volumes/SSD Samsung/theorem-valkey}"
PORT="${THEOREM_VALKEY_PORT:-6391}"
DAEMONIZE="${THEOREM_VALKEY_DAEMONIZE:-false}"
PID_FILE="${THEOREM_VALKEY_PID_FILE:-$DATA_DIR/theorem-valkey-local.pid}"
LOG_FILE="${THEOREM_VALKEY_LOG_FILE:-$DATA_DIR/theorem-valkey-local.log}"

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
SERVER_ARGS=(
  --dir "$DATA_DIR" \
  --port "$PORT" \
  --appendonly yes \
  --appendfsync everysec \
  --save ""
)

if [[ "$DAEMONIZE" == "true" || "$DAEMONIZE" == "1" || "$DAEMONIZE" == "yes" ]]; then
  SERVER_ARGS+=(--daemonize yes --pidfile "$PID_FILE" --logfile "$LOG_FILE")
fi

exec "$SERVER" "${SERVER_ARGS[@]}"
