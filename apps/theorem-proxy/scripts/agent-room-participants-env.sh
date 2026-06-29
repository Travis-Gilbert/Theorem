#!/usr/bin/env bash

THEOREM_PROXY_ENV_DIR="${THEOREM_PROXY_ENV_DIR:-$HOME/.theorem-proxy}"

load_provider_env() {
  local name="$1"
  local path="$THEOREM_PROXY_ENV_DIR/$name.env"
  if [ ! -f "$path" ]; then
    echo "agent-room: skipped $path (not found)" >&2
    return 0
  fi

  set -a
  # shellcheck source=/dev/null
  . "$path"
  set +a
  echo "agent-room: loaded $name env from $path" >&2
}

load_provider_env qwen
load_provider_env mistral

export THEOREM_AGENT_HEADS="${THEOREM_AGENT_HEADS:-qwen,mistral}"
export THEOREM_HEAD_INVOKER="${THEOREM_HEAD_INVOKER:-real}"
export THEOREM_AGENT_ROOM_RUNNER="${THEOREM_AGENT_ROOM_RUNNER:-1}"
export THEOREM_AGENT_TENANT_SLUG="${THEOREM_AGENT_TENANT_SLUG:-Travis-Gilbert}"
export THEOREM_AGENT_ROOM_ID="${THEOREM_AGENT_ROOM_ID:-general}"
export QWEN_MODEL="${QWEN_MODEL:-qwen3.7-max}"
export MISTRAL_MODEL="${MISTRAL_MODEL:-mistral-small-latest}"

if [ -n "${DASHSCOPE_API_KEY:-}" ] && [ -z "${QWEN_API_KEY:-}" ]; then
  export QWEN_API_KEY="$DASHSCOPE_API_KEY"
fi

# These are proxy/Codex routing variables, not harness provider-head inputs.
unset THEOREM_PROXY_OPENAI_UPSTREAM
unset THEOREM_PROXY_OPENAI_UPSTREAM_API_KEY
unset THEOREM_CODEX_MODEL
unset THEOREM_CODEX_PROVIDER_NAME

echo "agent-room: heads=${THEOREM_AGENT_HEADS} room=${THEOREM_AGENT_ROOM_ID} tenant=${THEOREM_AGENT_TENANT_SLUG}" >&2
