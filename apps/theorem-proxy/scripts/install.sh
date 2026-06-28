#!/usr/bin/env sh
# Install theorem-proxy from this checkout (roadmap C.1 one-command connect).
# Builds the crate in release and installs the `theorem-proxy` binary onto PATH via
# cargo. CPU-only: no model is downloaded. After install:
#   theorem-proxy wrap -- claude     # proxy up + Claude Code pointed at it
#   theorem-proxy doctor             # check the local stack
#
# The brew tap + `curl ... | sh`-from-a-release distribution is the remaining C.1 piece
# (needs a published release/formula); this is the from-source path.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
CRATE="$(cd "$HERE/.." && pwd)"

echo "installing theorem-proxy from $CRATE ..."
cargo install --path "$CRATE" --force

BIN="$(command -v theorem-proxy 2>/dev/null || echo "$HOME/.cargo/bin/theorem-proxy")"
echo
echo "installed: $BIN"
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) echo "note: add cargo bin to PATH -> export PATH=\"\$HOME/.cargo/bin:\$PATH\"" ;;
esac
echo "connect Claude Code:  theorem-proxy wrap -- claude"
echo "check the stack:      theorem-proxy doctor"
