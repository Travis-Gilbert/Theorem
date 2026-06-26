#!/usr/bin/env bash
set -euo pipefail

IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly CRATE_ROOT="$REPO_ROOT/rustyredcore_THG/crates"

mode="dry-run"
selected_layer="all"
include_heavy=false
allow_dirty=false
no_verify=false

usage() {
    cat >&2 <<'USAGE'
Usage: scripts/release-crates.sh [options]

Options:
  --mode dry-run|package|publish   Release command to run. Default: dry-run.
  --layer 0|1|2|3|all              Layer to run. Default: all (0,1,2).
  --include-heavy                  Include layer 3 when --layer all is used.
  --allow-dirty                    Permit dirty crate files for dry-run/package.
  --no-verify                      Only for --mode package; assemble without build verification.
  -h, --help                       Show this help.

Actual crates.io upload is guarded: set CONFIRM_PUBLISH=yes with --mode publish.
USAGE
}

log() {
    echo "release-crates: $*" >&2
}

fail() {
    echo "release-crates: error: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)
            [[ $# -ge 2 ]] || fail "--mode requires a value"
            mode="$2"
            shift 2
            ;;
        --layer)
            [[ $# -ge 2 ]] || fail "--layer requires a value"
            selected_layer="$2"
            shift 2
            ;;
        --include-heavy)
            include_heavy=true
            shift
            ;;
        --allow-dirty)
            allow_dirty=true
            shift
            ;;
        --no-verify)
            no_verify=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage
            fail "unknown argument: $1"
            ;;
    esac
done

case "$mode" in
    dry-run|package|publish) ;;
    *) fail "--mode must be dry-run, package, or publish" ;;
esac

case "$selected_layer" in
    0|1|2|3|all) ;;
    *) fail "--layer must be 0, 1, 2, 3, or all" ;;
esac

if [[ "$mode" == "publish" ]]; then
    [[ "${CONFIRM_PUBLISH:-}" == "yes" ]] || fail "set CONFIRM_PUBLISH=yes to publish"
    [[ "$allow_dirty" == "false" ]] || fail "--allow-dirty is not allowed with --mode publish"
fi

if [[ "$no_verify" == "true" && "$mode" != "package" ]]; then
    fail "--no-verify is only valid with --mode package"
fi

readonly -a LAYER_0=(
    theorem-harness-core
    prose-check
    design-check
)

readonly -a LAYER_1=(
    rustyred-thg-core
)

readonly -a LAYER_2=(
    rustyred-thg-memory
    rustyred-thg-affordances
    rustyred-thg-connectors
    theorem-dispatch
    theorem-receiver
    theorem-browser-agent
    theorem-harness-runtime
)

# Heavy app/server layer. This is intentionally opt-in because it pulls a broad
# Theorem runtime forest and should follow the smaller library crate uploads.
readonly -a LAYER_3=(
    rustyred-thg-server
)

crates_for_layer() {
    local layer=$1
    case "$layer" in
        0) printf '%s\n' "${LAYER_0[@]}" ;;
        1) printf '%s\n' "${LAYER_1[@]}" ;;
        2) printf '%s\n' "${LAYER_2[@]}" ;;
        3) printf '%s\n' "${LAYER_3[@]}" ;;
        *) fail "unknown layer: $layer" ;;
    esac
}

selected_layers() {
    if [[ "$selected_layer" == "all" ]]; then
        printf '%s\n' 0 1 2
        if [[ "$include_heavy" == "true" ]]; then
            printf '%s\n' 3
        fi
    else
        printf '%s\n' "$selected_layer"
    fi
}

crate_is_dirty() {
    local crate_name=$1
    local crate_path="rustyredcore_THG/crates/$crate_name"
    [[ -n "$(git -C "$REPO_ROOT" status --porcelain -- "$crate_path")" ]]
}

run_crate() {
    local crate_name=$1
    local manifest="$CRATE_ROOT/$crate_name/Cargo.toml"
    [[ -f "$manifest" ]] || fail "missing manifest: $manifest"

    if crate_is_dirty "$crate_name" && [[ "$allow_dirty" == "false" ]]; then
        fail "$crate_name has uncommitted changes; commit them or rerun with --allow-dirty for non-publish modes"
    fi

    log "$mode $crate_name"
    case "$mode" in
        dry-run)
            local -a args=(publish --manifest-path "$manifest" --dry-run)
            if [[ "$allow_dirty" == "true" ]]; then
                args+=(--allow-dirty)
            fi
            cargo "${args[@]}"
            ;;
        package)
            local -a args=(package --manifest-path "$manifest")
            if [[ "$allow_dirty" == "true" ]]; then
                args+=(--allow-dirty)
            fi
            if [[ "$no_verify" == "true" ]]; then
                args+=(--no-verify)
            fi
            cargo "${args[@]}"
            ;;
        publish)
            cargo publish --manifest-path "$manifest"
            ;;
    esac
}

main() {
    local layer crate_name
    while IFS= read -r layer; do
        log "layer $layer"
        while IFS= read -r crate_name; do
            run_crate "$crate_name"
        done < <(crates_for_layer "$layer")
    done < <(selected_layers)
}

main "$@"
