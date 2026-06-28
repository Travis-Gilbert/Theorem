#!/usr/bin/env bash
# One-shot: bring up the local node and run Claude Code through the proxy with ambient
# memory (roadmap G3). Tears the node down on exit.
#
#   scripts/start-proxied-session.sh                 # node + proxy + `claude`
#   THEOREM_SEED=1 scripts/start-proxied-session.sh  # also (re)seed the node's graph memory
#   scripts/start-proxied-session.sh claude -p "hi"  # args pass through to the wrapped cmd
#
# Store: the node runs embedded RedCore (RustyRed) -- the canonical local store. Valkey is
# NOT used in this mode (it is only the RUSTY_RED_MODE=redis compatibility path), so it is
# not started here. Ambient memory defaults to the memory directory (fast, relevant);
# THEOREM_USE_NODE_MEMORY=1 switches to the node's graph retrieval.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NODE_PORT="${RUSTY_RED_PORT:-8380}"
PROXY_PORT="${THEOREM_PROXY_PORT:-8788}"
PROXY="${THEOREM_PROXY_BIN:-$HOME/.cargo/bin/theorem-proxy}"
MEM_DIR="${THEOREM_MEMORY_DIR:-$HOME/.claude/projects/-Users-travisgilbert-Tech-Dev-Local-Creative-Website-Theorem/memory}"

cleanup() { pkill -f "rustyred-thg-server" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM

up() {
  for _ in $(seq 1 300); do
    curl -fsS "$1" >/dev/null 2>&1 && return 0
    perl -e 'select(undef,undef,undef,0.2)'
  done
  return 1
}

# Local node (embedded RedCore = RustyRed). Serves the harness MCP tools the plugin now
# points at, plus the optional node graph memory.
if ! curl -fsS "http://127.0.0.1:$NODE_PORT/ready" >/dev/null 2>&1; then
  RUSTY_RED_PORT="$NODE_PORT" bash "$HERE/node-local.sh" >/tmp/theorem-node.log 2>&1 &
fi
up "http://127.0.0.1:$NODE_PORT/ready" || {
  echo "node failed to start; see /tmp/theorem-node.log" >&2
  exit 1
}
echo "node up (embedded RedCore) at 127.0.0.1:$NODE_PORT"

# Optional: seed the node's graph memory. The node's hippo_retrieve / index_context need
# it; the default --memory-dir ambient path does not. First run is slow (index warm).
if [ "${THEOREM_SEED:-0}" = "1" ]; then
  THEOREM_NODE_URL="http://127.0.0.1:$NODE_PORT/mcp" python3 "$HERE/seed-node.py" \
    || echo "seed had issues (continuing)" >&2
fi

[ "$#" -eq 0 ] && set -- claude
if [ "${THEOREM_USE_NODE_MEMORY:-0}" = "1" ]; then
  SOURCE=(--memory-url "http://127.0.0.1:$NODE_PORT/mcp")
  echo "launching '$*' (ambient memory: node graph) ..."
else
  SOURCE=(--memory-dir "$MEM_DIR")
  echo "launching '$*' (ambient memory: $MEM_DIR) ..."
fi
# Not `exec`: keep this shell alive so the EXIT trap tears the node down when the wrapped
# session ends.
"$PROXY" wrap --port "$PROXY_PORT" "${SOURCE[@]}" -- "$@"
