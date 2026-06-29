#!/usr/bin/env bash
# One-shot: bring up the local RustyRed node, start theorem-proxy, and launch Codex
# with its OpenAI Responses traffic pointed through the proxy.
#
#   scripts/start-proxied-codex-session.sh
#   THEOREM_SEED=1 scripts/start-proxied-codex-session.sh
#   scripts/start-proxied-codex-session.sh exec "summarize AGENTS.md"
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE_PORT="${RUSTY_RED_PORT:-8380}"
PROXY_PORT="${THEOREM_PROXY_PORT:-8788}"
PROXY="${THEOREM_PROXY_BIN:-$HOME/.cargo/bin/theorem-proxy}"
CODEX="${CODEX_BIN:-codex}"
MEM_DIR="${THEOREM_MEMORY_DIR:-$HOME/.claude/projects/-Users-travisgilbert-Tech-Dev-Local-Creative-Website-Theorem/memory}"
OWN_NODE=0
OWN_PROXY=0

cleanup() {
  if [ "$OWN_PROXY" = "1" ]; then
    pkill -f "theorem-proxy proxy --port $PROXY_PORT" >/dev/null 2>&1 || true
  fi
  if [ "$OWN_NODE" = "1" ]; then
    pkill -f "rustyred-thg-server" >/dev/null 2>&1 || true
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
  RUSTY_RED_PORT="$NODE_PORT" bash "$HERE/node-local.sh" >/tmp/theorem-node.log 2>&1 &
  OWN_NODE=1
fi
up "http://127.0.0.1:$NODE_PORT/ready" || {
  echo "node failed to start; see /tmp/theorem-node.log" >&2
  exit 1
}
echo "node up (embedded RedCore) at 127.0.0.1:$NODE_PORT"

if [ "${THEOREM_SEED:-0}" = "1" ]; then
  THEOREM_NODE_URL="http://127.0.0.1:$NODE_PORT/mcp" python3 "$HERE/seed-node.py" \
    || echo "seed had issues (continuing)" >&2
fi

if [ "${THEOREM_USE_NODE_MEMORY:-0}" = "1" ]; then
  SOURCE=(--memory-url "http://127.0.0.1:$NODE_PORT/mcp")
  echo "ambient memory: node graph"
else
  SOURCE=(--memory-dir "$MEM_DIR")
  echo "ambient memory: $MEM_DIR"
fi

if ! curl -fsS "http://127.0.0.1:$PROXY_PORT/healthz" >/dev/null 2>&1; then
  "$PROXY" proxy --port "$PROXY_PORT" "${SOURCE[@]}" >/tmp/theorem-proxy-codex.log 2>&1 &
  OWN_PROXY=1
fi
up "http://127.0.0.1:$PROXY_PORT/healthz" || {
  echo "proxy failed to start; see /tmp/theorem-proxy-codex.log" >&2
  exit 1
}
echo "proxy up at 127.0.0.1:$PROXY_PORT"
echo "launching Codex through http://127.0.0.1:$PROXY_PORT/v1 ..."

"$CODEX" -c "openai_base_url=\"http://127.0.0.1:$PROXY_PORT/v1\"" "$@"
