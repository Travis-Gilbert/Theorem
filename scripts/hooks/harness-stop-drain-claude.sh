#!/usr/bin/env bash
# Stop hook: warm Claude Code heads drain ask/block harness mentions at turn
# boundary. Cold heads are handled by the server wake router.
set -euo pipefail

main() {
  if [[ "${THEOREM_HARNESS_STOP_HOOK_ACTIVE:-0}" == "1" ]]; then
    exit 0
  fi
  export THEOREM_HARNESS_STOP_HOOK_ACTIVE=1

  local base_url="${THEOREM_HARNESS_HTTP_URL:-}"
  if [[ -z "$base_url" ]]; then
    exit 0
  fi
  if ! command -v curl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
    exit 0
  fi

  local actor="${THEOREM_HARNESS_ACTOR:-claude-code}"
  local tenant="${THEOREM_HARNESS_TENANT:-Travis-Gilbert}"
  local limit="${THEOREM_HARNESS_STOP_MENTION_LIMIT:-20}"
  local max_blocks="${THEOREM_HARNESS_STOP_MAX_BLOCKS_PER_CYCLE:-1}"
  local project_dir="${CLAUDE_PROJECT_DIR:-$PWD}"
  local state_dir="$project_dir/.theorem"
  local state_file="$state_dir/stop-hook-blocks-$actor.count"

  mkdir -p "$state_dir"

  local tenant_q
  tenant_q="$(jq -rn --arg value "$tenant" '$value|@uri')"
  local actor_q
  actor_q="$(jq -rn --arg value "$actor" '$value|@uri')"

  local response
  response="$(
    curl -fsS \
      "$base_url/harness/actors/$actor_q/mentions?tenant=$tenant_q&urgencies=ask,block&consume=true&limit=$limit" \
      2>/dev/null || true
  )"
  if [[ -z "$response" ]]; then
    exit 0
  fi

  local reason
  reason="$(
    printf '%s' "$response" | jq -cer '
      .mentions
      | if length == 0 then empty else
          .[0] as $mention
          | "Harness mention pending for " + $mention.actor_id + " in " + $mention.room_id
            + " (" + $mention.urgency + "): " + $mention.message
            + "\nReact via /theorems-harness:harness."
        end
    ' 2>/dev/null || true
  )"
  if [[ -z "$reason" ]]; then
    rm -f "$state_file"
    exit 0
  fi

  local block_count="0"
  if [[ -f "$state_file" ]]; then
    block_count="$(tr -cd '0-9' < "$state_file" || true)"
    block_count="${block_count:-0}"
  fi
  if (( block_count >= max_blocks )); then
    exit 0
  fi
  printf '%s\n' "$((block_count + 1))" > "$state_file"

  local reason_json
  reason_json="$(jq -rn --arg reason "$reason" '$reason')"
  printf '{"decision":"block","reason":%s}\n' "$reason_json"
}

main "$@"
