#!/usr/bin/env bash
# Shared helpers for Theorem Carry local memory hooks.
#
# These helpers are used by lifecycle hooks, so public entrypoints must remain
# fail-open. Library functions return non-zero on failure; callers decide whether
# to ignore the error.

set -euo pipefail
IFS=$'\n\t'

carry_script_dir() {
    cd "$(dirname "${BASH_SOURCE[0]}")" && pwd
}

carry_repo_root() {
    if [[ -n "${THEOREM_REPO:-}" && -d "$THEOREM_REPO" ]]; then
        printf '%s\n' "$THEOREM_REPO"
        return 0
    fi
    if command -v git >/dev/null 2>&1; then
        git rev-parse --show-toplevel 2>/dev/null && return 0
    fi
    printf '%s\n' "${PWD:-.}"
}

carry_repo_origin() {
    local repo=$1
    if command -v git >/dev/null 2>&1; then
        git -C "$repo" remote get-url origin 2>/dev/null || true
    fi
}

carry_repo_sha() {
    local repo=$1
    if command -v git >/dev/null 2>&1; then
        git -C "$repo" rev-parse HEAD 2>/dev/null || true
    fi
}

carry_repo_identity() {
    local repo=$1
    local origin
    origin="$(carry_repo_origin "$repo")"
    if [[ -n "$origin" ]]; then
        printf '%s\n' "$origin"
        return 0
    fi
    printf '%s\n' "$repo"
}

carry_repo_slug() {
    local identity=$1
    python3 - "$identity" <<'PY'
import hashlib
import re
import sys

identity = sys.argv[1]
base = identity.rstrip("/").split("/")[-1] or "repo"
base = re.sub(r"\.git$", "", base)
base = re.sub(r"[^A-Za-z0-9._-]+", "-", base).strip("-") or "repo"
digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:16]
print(f"{base}-{digest}")
PY
}

carry_home() {
    if [[ -n "${THEOREM_CARRY_HOME:-}" ]]; then
        printf '%s\n' "$THEOREM_CARRY_HOME"
        return 0
    fi
    if [[ -n "${XDG_DATA_HOME:-}" ]]; then
        printf '%s\n' "$XDG_DATA_HOME/theorem-carry"
        return 0
    fi
    printf '%s\n' "$HOME/.local/share/theorem-carry"
}

carry_data_dir() {
    local repo identity slug
    repo="$(carry_repo_root)"
    identity="$(carry_repo_identity "$repo")"
    slug="$(carry_repo_slug "$identity")"
    printf '%s/projects/%s\n' "$(carry_home)" "$slug"
}

carry_mcp_url() {
    if [[ -n "${THEOREM_CARRY_MCP_URL:-}" ]]; then
        printf '%s\n' "$THEOREM_CARRY_MCP_URL"
        return 0
    fi
    if [[ -n "${THEOREM_MCP_URL:-}" ]]; then
        printf '%s\n' "$THEOREM_MCP_URL"
        return 0
    fi
    printf 'http://127.0.0.1:%s/mcp\n' "${RUSTY_RED_PORT:-8380}"
}

carry_agent() {
    if [[ -n "${THEOREM_CARRY_AGENT:-}" ]]; then
        printf '%s\n' "$THEOREM_CARRY_AGENT"
        return 0
    fi
    if [[ -n "${CLAUDECODE:-}" || -n "${CLAUDE_CODE_ENTRYPOINT:-}" ]]; then
        printf 'claude-code\n'
        return 0
    fi
    printf 'codex\n'
}

carry_now_iso() {
    date -u +"%Y-%m-%dT%H:%M:%SZ"
}

carry_json_quote() {
    python3 - "$1" <<'PY'
import json
import sys

print(json.dumps(sys.argv[1]))
PY
}

carry_call_tool() {
    local name=$1
    local arguments_json
    if [[ $# -ge 2 && -n "${2:-}" ]]; then
        arguments_json=$2
    else
        arguments_json="{}"
    fi
    local url token timeout
    url="$(carry_mcp_url)"
    token="${THEOREM_API_TOKEN:-}"
    timeout="${THEOREM_CARRY_TIMEOUT_SECS:-4}"
    python3 - "$url" "$token" "$name" "$arguments_json" "$timeout" <<'PY'
import json
import sys
import urllib.error
import urllib.request

url, token, name, arguments_json, timeout_raw = sys.argv[1:6]
try:
    arguments = json.loads(arguments_json or "{}")
except json.JSONDecodeError as exc:
    print(json.dumps({"error": f"invalid arguments JSON: {exc}"}))
    sys.exit(2)
try:
    timeout = float(timeout_raw)
except ValueError:
    timeout = 4.0

payload = {
    "jsonrpc": "2.0",
    "id": name,
    "method": "tools/call",
    "params": {"name": name, "arguments": arguments},
}
request = urllib.request.Request(
    url,
    data=json.dumps(payload).encode("utf-8"),
    method="POST",
    headers={"Content-Type": "application/json"},
)
if token:
    request.add_header("Authorization", "Bearer " + token)
try:
    with urllib.request.urlopen(request, timeout=timeout) as response:
        body = response.read().decode("utf-8")
except (OSError, urllib.error.URLError, TimeoutError) as exc:
    print(json.dumps({"error": str(exc)}))
    sys.exit(1)
print(body)
try:
    parsed = json.loads(body)
except json.JSONDecodeError:
    sys.exit(1)
if "error" in parsed:
    sys.exit(1)
result = parsed.get("result", {})
if isinstance(result, dict) and result.get("isError"):
    sys.exit(1)
PY
}
