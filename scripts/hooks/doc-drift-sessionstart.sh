#!/usr/bin/env bash
# SessionStart hook: prime the model with current doc-map status.
# hookEventName MUST equal "SessionStart" (the firing event) or CC drops the context.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
full="$("$ROOT/scripts/check-doc-drift.sh" --full 2>/dev/null || true)"
msg="Doc-map status (Theorem). $full
If this session adds, renames, or removes a crate/app, update the crate/app table in CLAUDE.md and the README 'Last sync' line BEFORE ending, then run scripts/check-doc-drift.sh --refresh. See docs/site/guides/doc-update-protocol.md."
# JSON-encode msg
enc=$(printf '%s' "$msg" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' 2>/dev/null || printf '"%s"' "$(printf '%s' "$msg" | tr '\n' ' ' | sed 's/"/\\"/g')")
printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":%s}}\n' "$enc"
exit 0
