#!/usr/bin/env bash
# Capture Claude Code or Codex lifecycle activity into the local RustyRed node.
# Hook entrypoints must be fail-open: the final command always exits 0.

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=carry-local.sh
source "$SCRIPT_DIR/carry-local.sh"

capture_main() {
    local event_arg=${1:-}
    local payload_file args_file event_file response_file
    payload_file="$(mktemp -t theorem-carry-payload.XXXXXX)"
    args_file="$(mktemp -t theorem-carry-args.XXXXXX)"
    event_file="$(mktemp -t theorem-carry-event.XXXXXX)"
    response_file="$(mktemp -t theorem-carry-response.XXXXXX)"
    trap 'rm -f "$payload_file" "$args_file" "$event_file" "$response_file"' RETURN

    cat >"$payload_file" || true

    local repo identity slug sha agent now
    repo="$(carry_repo_root)"
    identity="$(carry_repo_identity "$repo")"
    slug="$(carry_repo_slug "$identity")"
    sha="$(carry_repo_sha "$repo")"
    agent="$(carry_agent)"
    now="$(carry_now_iso)"

    python3 - "$event_arg" "$payload_file" "$args_file" "$event_file" \
        "$repo" "$identity" "$slug" "$sha" "$agent" "$now" <<'PY'
import hashlib
import json
import os
import re
import sys
from pathlib import Path

event_arg, payload_path, args_path, event_path, repo, identity, slug, sha, agent, now = sys.argv[1:11]
raw = Path(payload_path).read_text(errors="replace")
try:
    payload = json.loads(raw) if raw.strip() else {}
except json.JSONDecodeError:
    payload = {"raw_preview": raw[:500], "parse_error": True}

def first_text(*keys):
    for key in keys:
        value = payload.get(key) if isinstance(payload, dict) else None
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""

def walk(value, parent_key=""):
    if isinstance(value, dict):
        for key, child in value.items():
            yield from walk(child, key)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child, parent_key)
    elif isinstance(value, str):
        yield parent_key, value

event = event_arg or first_text("hook_event_name", "hookEventName", "event", "event_name") or "Manual"
session_id = (
    first_text("session_id", "sessionId", "conversation_id", "conversationId")
    or os.environ.get("CODEX_SESSION_ID", "")
    or os.environ.get("CLAUDE_SESSION_ID", "")
)
tool = first_text("tool_name", "toolName", "tool", "name")
prompt = first_text("prompt", "user_prompt", "userPrompt", "message")
cwd = first_text("cwd", "workspace", "worktree") or os.getcwd()

paths = []
commands = []
for key, value in walk(payload):
    lowered = key.lower()
    if any(part in lowered for part in ("path", "file", "dir", "cwd")):
        if "/" in value or "." in value:
            paths.append(value)
    if lowered in {"command", "cmd"} and len(value) <= 500:
        commands.append(value)

deduped_paths = []
seen = set()
for path in paths:
    normalized = path.strip()
    if normalized and normalized not in seen:
        seen.add(normalized)
        deduped_paths.append(normalized)
deduped_paths = deduped_paths[:20]

payload_keys = sorted(payload.keys()) if isinstance(payload, dict) else []
summary_bits = [event]
if tool:
    summary_bits.append(tool)
if deduped_paths:
    summary_bits.append(", ".join(deduped_paths[:3]))
summary = " | ".join(summary_bits)

content_lines = [
    f"event: {event}",
    f"agent: {agent}",
    f"repo: {repo}",
    f"repo_identity: {identity}",
    f"repo_sha: {sha}",
    f"session_id: {session_id}",
    f"cwd: {cwd}",
]
if tool:
    content_lines.append(f"tool: {tool}")
if deduped_paths:
    content_lines.append("paths:")
    content_lines.extend(f"- {path}" for path in deduped_paths)
if prompt and event.lower() == "userpromptsubmit":
    content_lines.append("prompt:")
    content_lines.append(prompt[:1000])
elif commands:
    content_lines.append("command_preview:")
    content_lines.append(commands[0][:500])

stable = "|".join([slug, session_id, event, now, raw[:1000]])
doc_hash = hashlib.sha256(stable.encode("utf-8")).hexdigest()[:24]
doc_id = f"carry:{slug}:{doc_hash}"
metadata = {
    "source": "theorem-carry",
    "hook_event": event,
    "agent": agent,
    "session_id": session_id,
    "repo_root": repo,
    "repo_identity": identity,
    "repo_slug": slug,
    "repo_sha": sha,
    "cwd": cwd,
    "file_paths": deduped_paths,
    "tool": tool,
    "observed_at": now,
    "payload_keys": payload_keys,
    "payload_parse_error": bool(payload.get("parse_error")) if isinstance(payload, dict) else False,
}
args = {
    "actor": agent,
    "session_id": session_id,
    "origin_surface": "theorem-carry-hook",
    "project_slug": slug,
    "doc_id": doc_id,
    "kind": "session_observation",
    "title": f"Theorem Carry {event}",
    "content": "\n".join(content_lines),
    "summary": summary,
    "gist": summary,
    "tags": ["theorem-carry", "session-observation", event],
    "status": "active",
    "memory_node_type": "observation",
    "metadata": metadata,
    "event_id": doc_id,
    "created_at": now,
    "updated_at": now,
}
Path(args_path).write_text(json.dumps(args, sort_keys=True))
Path(event_path).write_text(event)
PY

    if ! carry_call_tool "upsert_note" "$(cat "$args_file")" >"$response_file" 2>/dev/null; then
        return 0
    fi

    local event
    event="$(cat "$event_file")"
    case "$event" in
        Stop | SessionEnd | PreCompact)
            write_session_summary "$args_file" >/dev/null 2>&1 || true
            ;;
    esac
}

write_session_summary() {
    local observation_args_file=$1
    local summary_args
    summary_args="$(python3 - "$observation_args_file" <<'PY'
import json
import sys
from pathlib import Path

observation = json.loads(Path(sys.argv[1]).read_text())
metadata = dict(observation.get("metadata") or {})
session_id = metadata.get("session_id", "")
repo_slug = metadata.get("repo_slug", observation.get("project_slug", "repo"))
event = metadata.get("hook_event", "Stop")
doc_id = f"carry:{repo_slug}:session-summary:{session_id or 'unknown'}"
content = "\n".join(
    [
        f"session_id: {session_id}",
        f"repo: {metadata.get('repo_root', '')}",
        f"repo_sha: {metadata.get('repo_sha', '')}",
        f"last_event: {event}",
        "summary: Session ended or compacted. Raw observations are stored as theorem-carry session_observation notes for this session.",
    ]
)
args = {
    "actor": metadata.get("agent", "theorem-carry"),
    "session_id": session_id,
    "origin_surface": "theorem-carry-hook",
    "project_slug": repo_slug,
    "doc_id": doc_id,
    "kind": "session_summary",
    "title": "Theorem Carry session summary",
    "content": content,
    "summary": "Session ended; theorem-carry observations are ready for recall.",
    "gist": "Session ended; theorem-carry observations are ready for recall.",
    "tags": ["theorem-carry", "session-summary"],
    "status": "active",
    "memory_node_type": "summary",
    "metadata": {**metadata, "source": "theorem-carry", "summary_marker": True},
    "event_id": doc_id,
    "updated_at": observation.get("updated_at", ""),
    "created_at": observation.get("created_at", ""),
}
print(json.dumps(args, sort_keys=True))
PY
)"
    carry_call_tool "upsert_note" "$summary_args"
}

capture_main "$@" || true
exit 0
