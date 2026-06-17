#!/usr/bin/env bash
# Theorem doc-drift checker.
# Compares crates/apps on disk against the navigation map (CLAUDE.md) and a
# baseline snapshot of known directories.
#
#   scripts/check-doc-drift.sh            # --new-only (default): dirs new since baseline
#   scripts/check-doc-drift.sh --full     # whole standing backlog vs CLAUDE.md
#   scripts/check-doc-drift.sh --refresh  # rewrite the baseline to current disk state
#
# Exit codes: 0 = clean (or advisory). 2 = drift AND THEOREM_DOC_DRIFT_BLOCK=1.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
MAP="CLAUDE.md"; README="README.md"
B_CR=".harness/doc-map-baseline.crates"; B_AP=".harness/doc-map-baseline.apps"
MODE="${1:---new-only}"

list_crates(){ ls -1 rustyredcore_THG/crates 2>/dev/null | sort; }
list_apps(){ ls -1 apps 2>/dev/null | sort; }
documented(){ grep -q "\`$1\`" "$MAP" 2>/dev/null; }
documented_app(){ grep -qE "apps/$1([/\` ]|$)" "$MAP" 2>/dev/null || grep -q "\`$1\`" "$MAP" 2>/dev/null; }

if [ "$MODE" = "--refresh" ]; then
  list_crates > "$B_CR"; list_apps > "$B_AP"
  echo "baseline refreshed: $(wc -l < "$B_CR") crates, $(wc -l < "$B_AP") apps"
  exit 0
fi

bk_cr=""; for n in $(list_crates); do documented "$n" || bk_cr="$bk_cr$n\n"; done; bk_cr=$(printf "%b" "$bk_cr" | sed "/^$/d")
bk_ap=""; for n in $(list_apps); do documented_app "$n" || bk_ap="$bk_ap$n\n"; done; bk_ap=$(printf "%b" "$bk_ap" | sed "/^$/d")
n_disk_cr=$(list_crates | wc -l | tr -d ' '); n_disk_ap=$(list_apps | wc -l | tr -d ' ')
n_bk_cr=$( [ -n "$bk_cr" ] && echo "$bk_cr" | wc -l | tr -d ' ' || echo 0 )
n_bk_ap=$( [ -n "$bk_ap" ] && echo "$bk_ap" | wc -l | tr -d ' ' || echo 0 )

new_cr=""; new_ap=""
[ -f "$B_CR" ] && new_cr=$(comm -23 <(list_crates) <(sort "$B_CR"))
[ -f "$B_AP" ] && new_ap=$(comm -23 <(list_apps) <(sort "$B_AP"))

sync_line=$(grep -nE "Last sync" "$README" 2>/dev/null | head -1 || true)

if [ "$MODE" = "--full" ]; then
  echo "== Theorem doc-drift (full) =="
  echo "crates on disk: $n_disk_cr   undocumented in CLAUDE.md: $n_bk_cr"
  [ -n "$bk_cr" ] && echo "$bk_cr" | sed 's/^/  - /'
  echo "apps on disk: $n_disk_ap   undocumented in CLAUDE.md: $n_bk_ap"
  [ -n "$bk_ap" ] && echo "$bk_ap" | sed 's/^/  - /'
  echo "README sync: ${sync_line:-(no 'Last sync' line found)}"
  exit 0
fi

# --new-only
if [ -z "$new_cr" ] && [ -z "$new_ap" ]; then
  echo "doc-map: no new crates/apps since baseline (standing backlog: $n_bk_cr crates, $n_bk_ap apps undocumented)"
  exit 0
fi
echo "DOC DRIFT: new directories since baseline are undocumented in CLAUDE.md:"
[ -n "$new_cr" ] && echo "$new_cr" | sed 's/^/  crate: /'
[ -n "$new_ap" ] && echo "$new_ap" | sed 's/^/  app:   /'
echo "Fix: add to the crate/app table in CLAUDE.md (+ README), then: scripts/check-doc-drift.sh --refresh"
[ "${THEOREM_DOC_DRIFT_BLOCK:-0}" = "1" ] && exit 2
exit 0
