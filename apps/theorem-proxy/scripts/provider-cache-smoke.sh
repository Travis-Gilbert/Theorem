#!/usr/bin/env bash
# Live provider prompt-cache smoke for theorem-proxy.
#
# This is intentionally not a Claude/Codex client smoke. It starts the local proxy,
# sends two identical Anthropic Messages requests with an explicit cache breakpoint,
# and asserts that the second provider response reports cached input tokens.
set -euo pipefail

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  echo "ANTHROPIC_API_KEY is required for the live provider cache smoke" >&2
  exit 2
fi

for tool in curl jq lsof; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool is required" >&2
    exit 2
  fi
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PORT="${THEOREM_PROXY_SMOKE_PORT:-8789}"
MODEL="${THEOREM_PROXY_SMOKE_MODEL:-claude-sonnet-4-5}"
MEMORY_URL="${THEOREM_PROXY_MEMORY_URL:-http://127.0.0.1:8380/mcp}"
TENANT="${THEOREM_PROXY_TENANT:-Travis-Gilbert}"
UPSTREAM="${THEOREM_PROXY_UPSTREAM:-https://api.anthropic.com}"
OUT_DIR="${THEOREM_PROXY_SMOKE_OUT_DIR:-/tmp/theorem-provider-cache-smoke}"
TARGET_DIR="${CARGO_TARGET_DIR:-/Volumes/SSD Samsung/theorem-recon-target/apps-theorem-proxy}"

mkdir -p "$OUT_DIR"
REQUEST="$OUT_DIR/request.json"
FIRST="$OUT_DIR/first.json"
SECOND="$OUT_DIR/second.json"
PROXY_LOG="$OUT_DIR/proxy.log"

STATIC_PREFIX="Theorem provider prompt-cache smoke static prefix."
for i in $(seq 1 1800); do
  STATIC_PREFIX+=" cache_anchor_${i}"
done

jq -n \
  --arg model "$MODEL" \
  --arg static_prefix "$STATIC_PREFIX" \
  '{
    model: $model,
    max_tokens: 8,
    system: [
      {
        type: "text",
        text: $static_prefix,
        cache_control: {type: "ephemeral"}
      }
    ],
    messages: [
      {
        role: "user",
        content: [
          {
            type: "text",
            text: "Reply with exactly: CACHE_SMOKE_OK"
          }
        ]
      }
    ]
  }' >"$REQUEST"

if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "port $PORT is already in use; set THEOREM_PROXY_SMOKE_PORT" >&2
  exit 2
fi

(
  cd "$APP_DIR"
  CARGO_TARGET_DIR="$TARGET_DIR" cargo run --bin theorem-proxy -- \
    proxy \
    --port "$PORT" \
    --upstream "$UPSTREAM" \
    --memory-url "$MEMORY_URL" \
    --tenant "$TENANT"
) >"$PROXY_LOG" 2>&1 &
PROXY_PID=$!
cleanup() {
  kill "$PROXY_PID" >/dev/null 2>&1 || true
  wait "$PROXY_PID" >/dev/null 2>&1 || true
}
trap cleanup EXIT

for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null

BETA_HEADER=()
if [[ -n "${ANTHROPIC_BETA:-}" ]]; then
  BETA_HEADER=(-H "anthropic-beta: ${ANTHROPIC_BETA}")
fi

call_provider() {
  local output="$1"
  curl -fsS "http://127.0.0.1:${PORT}/v1/messages" \
    -H "x-api-key: ${ANTHROPIC_API_KEY}" \
    -H "anthropic-version: 2023-06-01" \
    "${BETA_HEADER[@]}" \
    -H "content-type: application/json" \
    --data @"$REQUEST" >"$output"
}

call_provider "$FIRST"
sleep 1
call_provider "$SECOND"

FIRST_CREATE="$(jq -r '.usage.cache_creation_input_tokens // 0' "$FIRST")"
FIRST_READ="$(jq -r '.usage.cache_read_input_tokens // 0' "$FIRST")"
SECOND_CREATE="$(jq -r '.usage.cache_creation_input_tokens // 0' "$SECOND")"
SECOND_READ="$(jq -r '.usage.cache_read_input_tokens // 0' "$SECOND")"

printf 'first:  cache_creation_input_tokens=%s cache_read_input_tokens=%s\n' "$FIRST_CREATE" "$FIRST_READ"
printf 'second: cache_creation_input_tokens=%s cache_read_input_tokens=%s\n' "$SECOND_CREATE" "$SECOND_READ"

if (( SECOND_READ <= 0 )); then
  echo "provider cache did not report a read hit on the second request" >&2
  echo "responses saved under $OUT_DIR" >&2
  exit 1
fi

echo "provider prompt-cache smoke passed; responses saved under $OUT_DIR"
