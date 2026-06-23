#!/usr/bin/env bash
# Stop hook: when the session added a NEW crate/app not in the baseline AND
# THEOREM_DOC_DRIFT_BLOCK=1, block the stop so the model documents it first.
# Advisory (no block) otherwise. hookEventName for a Stop block is the decision form.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$("$ROOT/scripts/check-doc-drift.sh" --new-only 2>/dev/null || true)"
rc=$?
if [ "${THEOREM_DOC_DRIFT_BLOCK:-0}" = "1" ] && printf '%s' "$out" | grep -q "DOC DRIFT"; then
  reason=$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' 2>/dev/null || printf '"%s"' "$(printf '%s' "$out" | tr '\n' ' ' | sed 's/"/\\"/g')")
  printf '{"decision":"block","reason":%s}\n' "$reason"
  exit 0
fi
# advisory: surface to the user terminal, do not block
printf '%s' "$out" | grep -q "DOC DRIFT" && printf '%s\n' "$out" >&2
exit 0
