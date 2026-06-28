#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

if [[ $# -ne 1 ]]; then
    printf 'usage: %s <release-tag>\n' "$0" >&2
    exit 1
fi

tag=$1
version=${tag#rustyred-v}
repo=${THEOREM_GITHUB_REPO:-Travis-Gilbert/Theorem}
template_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

download_sha() {
    local target=$1
    local url="https://github.com/$repo/releases/download/$tag/rustyred-$target.tar.gz.sha256"
    local file="$tmp/$target.sha256"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$file"
    else
        wget -qO "$file" "$url"
    fi
    awk '{print $1}' "$file"
}

sha_aarch64_apple_darwin="$(download_sha aarch64-apple-darwin)"
sha_x86_64_apple_darwin="$(download_sha x86_64-apple-darwin)"
sha_aarch64_unknown_linux_gnu="$(download_sha aarch64-unknown-linux-gnu)"
sha_x86_64_unknown_linux_gnu="$(download_sha x86_64-unknown-linux-gnu)"

sed \
    -e "s/__TAG__/$tag/g" \
    -e "s/__VERSION__/$version/g" \
    -e "s/__SHA_AARCH64_APPLE_DARWIN__/$sha_aarch64_apple_darwin/g" \
    -e "s/__SHA_X86_64_APPLE_DARWIN__/$sha_x86_64_apple_darwin/g" \
    -e "s/__SHA_AARCH64_UNKNOWN_LINUX_GNU__/$sha_aarch64_unknown_linux_gnu/g" \
    -e "s/__SHA_X86_64_UNKNOWN_LINUX_GNU__/$sha_x86_64_unknown_linux_gnu/g" \
    "$template_dir/rustyred.rb.template"
