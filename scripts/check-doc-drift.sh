#!/usr/bin/env bash
# Theorem doc-drift checker.
# A crate/app is "documented" when it has a README.md at its base -- the doc that
# sits next to the code. CLAUDE.md is the navigation map, not the drift signal.
# Generate missing crate READMEs from Cargo.toml + //! docs with
# scripts/gen-crate-readmes.sh.
#
#   scripts/check-doc-drift.sh            # --new-only (default): new dirs lacking a base README
#   scripts/check-doc-drift.sh --full     # whole standing backlog (dirs lacking a base README)
#   scripts/check-doc-drift.sh --refresh  # rewrite the baseline to current disk state
#
# Exit codes: 0 = clean (or advisory). 2 = drift AND THEOREM_DOC_DRIFT_BLOCK=1.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
README="README.md"
B_CR=".harness/doc-map-baseline.crates"; B_AP=".harness/doc-map-baseline.apps"
MODE="${1:---new-only}"

list_crates(){ ls -1 rustyredcore_THG/crates 2>/dev/null | sort; }
list_apps(){ ls -1 apps 2>/dev/null | sort; }
documented(){ [ -f "rustyredcore_THG/crates/$1/README.md" ]; }
documented_app(){ [ -f "apps/$1/README.md" ]; }

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

# New since baseline AND still lacking a base README (a new crate that ships its
# own README is not drift).
new_cr=""; new_ap=""
if [ -f "$B_CR" ]; then for n in $(comm -23 <(list_crates) <(sort "$B_CR")); do documented "$n" || new_cr="$new_cr$n\n"; done; new_cr=$(printf "%b" "$new_cr" | sed "/^$/d"); fi
if [ -f "$B_AP" ]; then for n in $(comm -23 <(list_apps) <(sort "$B_AP")); do documented_app "$n" || new_ap="$new_ap$n\n"; done; new_ap=$(printf "%b" "$new_ap" | sed "/^$/d"); fi

sync_line=$(grep -nE "Last sync" "$README" 2>/dev/null | head -1 || true)

if [ "$MODE" = "--full" ]; then
  echo "== Theorem doc-drift (full) =="
  echo "crates on disk: $n_disk_cr   without a base README: $n_bk_cr"
  [ -n "$bk_cr" ] && echo "$bk_cr" | sed 's/^/  - /'
  echo "apps on disk: $n_disk_ap   without a base README: $n_bk_ap"
  [ -n "$bk_ap" ] && echo "$bk_ap" | sed 's/^/  - /'
  echo "README sync: ${sync_line:-(no 'Last sync' line found)}"
  exit 0
fi

# --new-only
if [ -z "$new_cr" ] && [ -z "$new_ap" ]; then
  echo "doc-map: no new crates/apps since baseline (standing backlog: $n_bk_cr crates, $n_bk_ap apps without a base README)"
  exit 0
fi
echo "DOC DRIFT: new directories since baseline lack a base README:"
[ -n "$new_cr" ] && echo "$new_cr" | sed 's/^/  crate: /'
[ -n "$new_ap" ] && echo "$new_ap" | sed 's/^/  app:   /'
echo "Fix: add a README.md at the crate/app base (crates: scripts/gen-crate-readmes.sh), then: scripts/check-doc-drift.sh --refresh"
[ "${THEOREM_DOC_DRIFT_BLOCK:-0}" = "1" ] && exit 2
exit 0
