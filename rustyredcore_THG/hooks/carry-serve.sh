#!/usr/bin/env bash
# Print a bounded local session-memory capsule from the local RustyRed node.
# This hook is fail-open and prints nothing when no useful memory is available.

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=carry-local.sh
source "$SCRIPT_DIR/carry-local.sh"

serve_main() {
    local repo identity slug sha agent task limit args_json response_file
    repo="$(carry_repo_root)"
    identity="$(carry_repo_identity "$repo")"
    slug="$(carry_repo_slug "$identity")"
    sha="$(carry_repo_sha "$repo")"
    agent="$(carry_agent)"
    task="${THEOREM_CONTEXT_TASK:-session memory for current repository}"
    limit="${THEOREM_CARRY_MEMORY_LIMIT:-6}"
    response_file="$(mktemp -t theorem-carry-serve.XXXXXX)"
    trap 'rm -f "$response_file"' RETURN

    args_json="$(python3 - "$agent" "$slug" "$repo" "$identity" "$sha" "$task" "$limit" <<'PY'
import json
import sys

agent, slug, repo, identity, sha, task, limit = sys.argv[1:8]
try:
    limit_int = max(1, min(int(limit), 12))
except ValueError:
    limit_int = 6
query = f"{task}\nrepo={repo}\nrepo_identity={identity}\nrepo_sha={sha}"
print(json.dumps({
    "actor": agent,
    "query": query,
    "limit": limit_int,
    "hydrate": False,
    "detail": "overview",
    "detail_top_k": limit_int,
    "project_slug": slug,
}))
PY
)"
    if ! carry_call_tool "observe" "$args_json" >"$response_file" 2>/dev/null; then
        return 0
    fi
    python3 - "$response_file" "$slug" "$sha" <<'PY'
import json
import sys
from pathlib import Path

raw = Path(sys.argv[1]).read_text()
slug = sys.argv[2]
sha = sys.argv[3]
try:
    envelope = json.loads(raw)
except json.JSONDecodeError:
    sys.exit(0)

result = envelope.get("result", {})
payload = result.get("structuredContent") if isinstance(result, dict) else None
if not payload and isinstance(result, dict):
    content = result.get("content")
    if isinstance(content, list):
        for item in content:
            text = item.get("text") if isinstance(item, dict) else None
            if not text:
                continue
            try:
                payload = json.loads(text)
                break
            except json.JSONDecodeError:
                continue
if not isinstance(payload, dict):
    payload = result if isinstance(result, dict) else {}

items = payload.get("recall_results") or payload.get("results") or []
if not isinstance(items, list) or not items:
    sys.exit(0)

print("## Session memory (Theorem Carry)")
print()
print(f"Project `{slug}` at `{sha[:12]}`.")
print()
for item in items[:6]:
    if not isinstance(item, dict):
        continue
    title = item.get("title") or item.get("doc_id") or item.get("id") or "memory"
    summary = item.get("summary") or item.get("gist") or item.get("content_preview") or item.get("content") or ""
    summary = " ".join(str(summary).split())
    if len(summary) > 260:
        summary = summary[:257] + "..."
    print(f"- {title}: {summary}")
PY
}

serve_main "$@" || true
exit 0
