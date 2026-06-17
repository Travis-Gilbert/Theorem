#!/usr/bin/env bash
# Generate a README.md for each crate in rustyredcore_THG/crates that lacks one,
# from its Cargo.toml `description` and `//!` module docs. Faithful surfacing of
# the author's own text; does not overwrite an existing README.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"
n=0
for d in rustyredcore_THG/crates/*/; do
  name=$(basename "$d"); [ -f "$d/README.md" ] && continue
  desc=$(grep -m1 -E '^description' "$d/Cargo.toml" 2>/dev/null | sed -E 's/^description[[:space:]]*=[[:space:]]*"//; s/"[[:space:]]*$//')
  doc=$(sed -n '1,40p' "$d/src/lib.rs" 2>/dev/null | grep -E '^//!' | sed -E 's#^//! ?##')
  { echo "# $name"; echo ""
    [ -n "$desc" ] && { echo "$desc"; echo ""; }
    [ -n "$doc" ] && { echo "## What it is"; echo ""; echo "$doc"; echo ""; }
    echo "## Build and test"; echo ""; echo '```bash'
    echo "cd rustyredcore_THG && cargo test -p $name"; echo '```'; echo ""
    echo "Part of the \`rustyredcore_THG\` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's \`Cargo.toml\` description and \`//!\` module docs; edit those and regenerate with \`scripts/gen-crate-readmes.sh\`."
  } > "$d/README.md"; n=$((n+1))
done
echo "generated $n READMEs"
