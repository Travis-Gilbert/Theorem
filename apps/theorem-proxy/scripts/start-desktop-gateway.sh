#!/usr/bin/env bash
# Start the local RustyRed node plus theorem-proxy for Claude Desktop 3P gateway
# mode. Claude Desktop should point at http://127.0.0.1:8788 with a harmless
# local gateway key; this process owns the real upstream Anthropic credential.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE_PORT="${RUSTY_RED_PORT:-8380}"
PROXY_PORT="${THEOREM_PROXY_PORT:-8788}"
PROXY="${THEOREM_PROXY_BIN:-$HOME/.cargo/bin/theorem-proxy}"
OWN_NODE=0
NODE_PID=""
NODE_LOG="${THEOREM_NODE_LOG:-$(mktemp -t theorem-node.XXXXXX.log)}"

if [ -n "${THEOREM_PROXY_UPSTREAM_API_KEY:-}" ] && [ -n "${THEOREM_PROXY_UPSTREAM_AUTH_TOKEN:-}" ]; then
  echo "set only one of THEOREM_PROXY_UPSTREAM_API_KEY or THEOREM_PROXY_UPSTREAM_AUTH_TOKEN" >&2
  exit 2
fi
if [ -z "${THEOREM_PROXY_UPSTREAM_API_KEY:-}" ] && [ -z "${THEOREM_PROXY_UPSTREAM_AUTH_TOKEN:-}" ]; then
  cat >&2 <<'EOF'
Claude Desktop gateway mode needs the proxy to own the upstream credential.

Set one of:
  export THEOREM_PROXY_UPSTREAM_API_KEY=...
  export THEOREM_PROXY_UPSTREAM_AUTH_TOKEN=...

For a Claude subscription/OAuth token, also set:
  export THEOREM_PROXY_UPSTREAM_BETA=oauth-2025-04-20
EOF
  exit 2
fi

cleanup() {
  if [ "$OWN_NODE" = "1" ] && [ -n "$NODE_PID" ] && kill -0 "$NODE_PID" >/dev/null 2>&1; then
    kill "$NODE_PID" >/dev/null 2>&1 || true
    wait "$NODE_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

up() {
  for _ in $(seq 1 300); do
    curl -fsS "$1" >/dev/null 2>&1 && return 0
    perl -e 'select(undef,undef,undef,0.2)'
  done
  return 1
}

if ! curl -fsS "http://127.0.0.1:$NODE_PORT/ready" >/dev/null 2>&1; then
  RUSTY_RED_PORT="$NODE_PORT" bash "$HERE/node-local.sh" >"$NODE_LOG" 2>&1 &
  NODE_PID=$!
  OWN_NODE=1
fi
up "http://127.0.0.1:$NODE_PORT/ready" || {
  echo "node failed to start; see $NODE_LOG" >&2
  exit 1
}
echo "node up (embedded RedCore) at 127.0.0.1:$NODE_PORT"

if [ "${THEOREM_SEED:-0}" = "1" ]; then
  THEOREM_NODE_URL="http://127.0.0.1:$NODE_PORT/mcp" python3 "$HERE/seed-node.py" \
    || echo "seed had issues (continuing)" >&2
fi

echo "Claude Desktop gateway: http://127.0.0.1:$PROXY_PORT"
echo "ambient memory: node graph at http://127.0.0.1:$NODE_PORT/mcp"
"$PROXY" proxy \
  --port "$PROXY_PORT" \
  --memory-url "http://127.0.0.1:$NODE_PORT/mcp"
