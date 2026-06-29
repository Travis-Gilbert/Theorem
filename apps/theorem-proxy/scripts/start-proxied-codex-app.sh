#!/usr/bin/env bash
# Start/reuse the local RustyRed node + theorem-proxy, then open Codex Desktop
# on a workspace. Current Codex Desktop does not honor `codex app -c
# openai_base_url=...` for model routing; this is an open-workspace helper until
# `theorem connect codex` owns user-level config.
#
#   apps/theorem-proxy/scripts/start-proxied-codex-app.sh
#   apps/theorem-proxy/scripts/start-proxied-codex-app.sh --restart-codex
#   THEOREM_SEED=1 apps/theorem-proxy/scripts/start-proxied-codex-app.sh /path/to/workspace
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
NODE_PORT="${RUSTY_RED_PORT:-8380}"
PROXY_PORT="${THEOREM_PROXY_PORT:-8788}"
PROXY="${THEOREM_PROXY_BIN:-$HOME/.cargo/bin/theorem-proxy}"
CODEX="${CODEX_BIN:-/Applications/Codex.app/Contents/Resources/codex}"
STATE_DIR="${THEOREM_PROXY_STATE_DIR:-$HOME/.theorem-proxy}"
RUN_DIR="$STATE_DIR/run"
LOG_DIR="$STATE_DIR/logs"
NODE_PID_FILE="$RUN_DIR/rustyred-node-$NODE_PORT.pid"
PROXY_PID_FILE="$RUN_DIR/theorem-proxy-$PROXY_PORT.pid"
PROXY_CONFIG_FILE="$RUN_DIR/theorem-proxy-$PROXY_PORT.config"
NODE_LOG="${THEOREM_NODE_LOG:-$LOG_DIR/rustyred-node-$NODE_PORT.log}"
PROXY_LOG="${THEOREM_PROXY_LOG:-$LOG_DIR/theorem-proxy-$PROXY_PORT.log}"
RESTART_CODEX=0
WORKSPACE="${THEOREM_CODEX_WORKSPACE:-$REPO_ROOT}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --restart-codex)
      RESTART_CODEX=1
      shift
      ;;
    --help|-h)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      WORKSPACE="$1"
      shift
      ;;
  esac
done

up() {
  for _ in $(seq 1 300); do
    curl -fsS "$1" >/dev/null 2>&1 && return 0
    perl -e 'select(undef,undef,undef,0.2)'
  done
  return 1
}

pid_alive() {
  [ -n "${1:-}" ] && kill -0 "$1" >/dev/null 2>&1
}

read_pid_file() {
  [ -f "$1" ] && sed -n '1p' "$1" || true
}

proxy_config_matches() {
  [ -f "$PROXY_CONFIG_FILE" ] && [ "$(cat "$PROXY_CONFIG_FILE")" = "$proxy_config" ]
}

resolve_proxy_bin() {
  if [ -x "$PROXY" ]; then
    printf '%s\n' "$PROXY"
    return 0
  fi
  command -v theorem-proxy
}

mkdir -p "$RUN_DIR" "$LOG_DIR"
PROXY="$(resolve_proxy_bin)"

if [ ! -x "$CODEX" ]; then
  echo "Codex launcher not found at $CODEX; set CODEX_BIN to override." >&2
  exit 1
fi

project_key="$(printf '%s' "$WORKSPACE" | sed 's#/#-#g; s# #-#g')"
legacy_project_key="$(printf '%s' "$WORKSPACE" | sed 's#/#-#g')"
default_mem_dir="$HOME/.claude/projects/$project_key/memory"
if [ ! -d "$default_mem_dir" ] && [ -d "$HOME/.claude/projects/$legacy_project_key/memory" ]; then
  default_mem_dir="$HOME/.claude/projects/$legacy_project_key/memory"
fi
mem_dir="${THEOREM_MEMORY_DIR:-$default_mem_dir}"
if [ "${THEOREM_USE_NODE_MEMORY:-0}" = "1" ]; then
  memory_kind="memory-url"
  memory_value="http://127.0.0.1:$NODE_PORT/mcp"
  source_args=(--memory-url "$memory_value")
  memory_label="node graph"
else
  memory_kind="memory-dir"
  memory_value="$mem_dir"
  source_args=(--memory-dir "$memory_value")
  memory_label="$mem_dir"
fi
proxy_config="$(printf 'memory_kind=%s\nmemory_value=%s' "$memory_kind" "$memory_value")"

if ! curl -fsS "http://127.0.0.1:$NODE_PORT/ready" >/dev/null 2>&1; then
  old_node_pid="$(read_pid_file "$NODE_PID_FILE")"
  if pid_alive "$old_node_pid"; then
    echo "node pid $old_node_pid is alive but /ready is not responding; see $NODE_LOG" >&2
    exit 1
  fi
  echo "starting local RustyRed node on 127.0.0.1:$NODE_PORT ..."
  RUSTY_RED_PORT="$NODE_PORT" nohup bash "$HERE/node-local.sh" >"$NODE_LOG" 2>&1 &
  echo "$!" >"$NODE_PID_FILE"
fi
up "http://127.0.0.1:$NODE_PORT/ready" || {
  echo "node failed to become ready; see $NODE_LOG" >&2
  exit 1
}

if [ "${THEOREM_SEED:-0}" = "1" ]; then
  THEOREM_NODE_URL="http://127.0.0.1:$NODE_PORT/mcp" THEOREM_MEMORY_DIR="$mem_dir" python3 "$HERE/seed-node.py" \
    || echo "seed had issues (continuing)" >&2
fi

if curl -fsS "http://127.0.0.1:$PROXY_PORT/healthz" >/dev/null 2>&1 && ! proxy_config_matches; then
  old_proxy_pid="$(read_pid_file "$PROXY_PID_FILE")"
  if ! pid_alive "$old_proxy_pid"; then
    echo "proxy is already healthy on 127.0.0.1:$PROXY_PORT, but its memory source is unknown or different." >&2
    echo "Stop that proxy or choose another THEOREM_PROXY_PORT before opening $WORKSPACE." >&2
    exit 1
  fi
  echo "restarting theorem-proxy on 127.0.0.1:$PROXY_PORT for memory: $memory_label ..."
  kill "$old_proxy_pid" >/dev/null 2>&1 || true
  wait "$old_proxy_pid" >/dev/null 2>&1 || true
  rm -f "$PROXY_PID_FILE" "$PROXY_CONFIG_FILE"
  for _ in $(seq 1 50); do
    curl -fsS "http://127.0.0.1:$PROXY_PORT/healthz" >/dev/null 2>&1 || break
    perl -e 'select(undef,undef,undef,0.2)'
  done
  if curl -fsS "http://127.0.0.1:$PROXY_PORT/healthz" >/dev/null 2>&1; then
    echo "proxy on 127.0.0.1:$PROXY_PORT did not stop; see $PROXY_LOG" >&2
    exit 1
  fi
fi

if ! curl -fsS "http://127.0.0.1:$PROXY_PORT/healthz" >/dev/null 2>&1; then
  old_proxy_pid="$(read_pid_file "$PROXY_PID_FILE")"
  if pid_alive "$old_proxy_pid"; then
    echo "proxy pid $old_proxy_pid is alive but /healthz is not responding; see $PROXY_LOG" >&2
    exit 1
  fi
  echo "starting theorem-proxy on 127.0.0.1:$PROXY_PORT ..."
  nohup "$PROXY" proxy --port "$PROXY_PORT" "${source_args[@]}" >"$PROXY_LOG" 2>&1 &
  echo "$!" >"$PROXY_PID_FILE"
fi
up "http://127.0.0.1:$PROXY_PORT/healthz" || {
  echo "proxy failed to become healthy; see $PROXY_LOG" >&2
  exit 1
}
printf '%s\n' "$proxy_config" >"$PROXY_CONFIG_FILE"

if [ "$RESTART_CODEX" = "1" ]; then
  echo "quitting existing Codex app so the workspace opens from a clean app process ..."
  osascript -e 'tell application "Codex" to quit' >/dev/null 2>&1 || true
  for _ in $(seq 1 100); do
    pgrep -x Codex >/dev/null 2>&1 || break
    perl -e 'select(undef,undef,undef,0.2)'
  done
elif pgrep -x Codex >/dev/null 2>&1; then
  echo "Codex is already running. If /status does not increment, rerun with --restart-codex." >&2
fi

echo "node:  http://127.0.0.1:$NODE_PORT"
echo "proxy: http://127.0.0.1:$PROXY_PORT/v1"
echo "memory: $memory_label"
echo "logs:  $LOG_DIR"
echo "opening Codex Desktop for $WORKSPACE ..."
echo "note: Desktop routing still requires user-level Codex config; verify with /status.openai_responses_seen."

"$CODEX" app "$WORKSPACE"
