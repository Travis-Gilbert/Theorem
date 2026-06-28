#!/usr/bin/env bash
# One-shot: bring up the local stack and run Claude Code through the proxy with ambient
# memory from the local node (roadmap G3). Tears the stack down on exit.
#
#   scripts/start-proxied-session.sh            # runs `claude` through the proxy
#   scripts/start-proxied-session.sh claude -p  # passes args through to the wrapped cmd
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE_PORT="${RUSTY_RED_PORT:-8380}"
VALKEY_PORT="${THEOREM_VALKEY_PORT:-6391}"
PROXY_PORT="${THEOREM_PROXY_PORT:-8788}"
PROXY="${THEOREM_PROXY_BIN:-$HOME/.cargo/bin/theorem-proxy}"

cleanup() {
  valkey-cli -p "$VALKEY_PORT" shutdown nosave >/dev/null 2>&1 || true
  pkill -f "rustyred-thg-server" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

up() {
  for _ in $(seq 1 150); do
    curl -fsS "$1" >/dev/null 2>&1 && return 0
    perl -e 'select(undef,undef,undef,0.2)'
  done
  return 1
}

# Valkey warm tier (best-effort).
if ! valkey-cli -p "$VALKEY_PORT" ping >/dev/null 2>&1; then
  THEOREM_VALKEY_PORT="$VALKEY_PORT" bash "$HERE/valkey-local.sh" >/tmp/theorem-valkey.log 2>&1 &
fi

# Local node (the harness substrate).
if ! curl -fsS "http://127.0.0.1:$NODE_PORT/ready" >/dev/null 2>&1; then
  RUSTY_RED_PORT="$NODE_PORT" bash "$HERE/node-local.sh" >/tmp/theorem-node.log 2>&1 &
fi
up "http://127.0.0.1:$NODE_PORT/ready" || {
  echo "node failed to start; see /tmp/theorem-node.log" >&2
  exit 1
}

# Seed the memory corpus (cheap; re-encode updates).
THEOREM_NODE_URL="http://127.0.0.1:$NODE_PORT/mcp" python3 "$HERE/seed-node.py" \
  || echo "seed step had issues (continuing)" >&2

[ "$#" -eq 0 ] && set -- claude

# Ambient memory source. Default: the memory directory (token-overlap relevance -- the
# proven, relevant path for keyword-rich memory atoms). Set THEOREM_USE_NODE_MEMORY=1 to
# use the node's graph retrieval instead (richer once a real embedder is configured; the
# node's default hash embedder gives weak relevance today). The node stays up either way
# for the harness MCP tools (coordination / hippo / index_context).
MEM_DIR="${THEOREM_MEMORY_DIR:-$HOME/.claude/projects/-Users-travisgilbert-Tech-Dev-Local-Creative-Website-Theorem/memory}"
if [ "${THEOREM_USE_NODE_MEMORY:-0}" = "1" ]; then
  SOURCE=(--memory-url "http://127.0.0.1:$NODE_PORT/mcp")
  echo "stack up. launching '$*' (ambient memory: local node graph) ..."
else
  SOURCE=(--memory-dir "$MEM_DIR")
  echo "stack up. launching '$*' (ambient memory: $MEM_DIR) ..."
fi
exec "$PROXY" wrap --port "$PROXY_PORT" "${SOURCE[@]}" -- "$@"
