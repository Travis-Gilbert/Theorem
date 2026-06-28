#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || true)"
readonly REPO_URL="${THEOREM_REPO_URL:-https://github.com/Travis-Gilbert/Theorem.git}"
readonly GITHUB_REPO="${THEOREM_GITHUB_REPO:-Travis-Gilbert/Theorem}"
readonly SOURCE_DIR="${THEOREM_SOURCE_DIR:-$HOME/.theorem/source}"
readonly INSTALL_DIR="${THEOREM_INSTALL_DIR:-$HOME/.local/bin}"
readonly THEOREM_HOME_DIR="${THEOREM_HOME:-$HOME/.theorem}"
readonly PID_FILE="$THEOREM_HOME_DIR/agentd.pid"
readonly OUT_LOG="$THEOREM_HOME_DIR/agentd.out.log"
readonly ERR_LOG="$THEOREM_HOME_DIR/agentd.err.log"

log() {
    printf '[theorem-install] %s\n' "$*" >&2
}

require_command() {
    local command_name=$1
    if ! command -v "$command_name" >/dev/null 2>&1; then
        log "$command_name is required"
        return 1
    fi
}

detect_release_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os:$arch" in
        Darwin:arm64 | Darwin:aarch64)
            printf 'aarch64-apple-darwin\n'
            ;;
        Darwin:x86_64)
            printf 'x86_64-apple-darwin\n'
            ;;
        Linux:x86_64 | Linux:amd64)
            printf 'x86_64-unknown-linux-gnu\n'
            ;;
        Linux:arm64 | Linux:aarch64)
            printf 'aarch64-unknown-linux-gnu\n'
            ;;
        *)
            log "unsupported release target: $os $arch"
            return 1
            ;;
    esac
}

download_file() {
    local url=$1
    local output=$2
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$output"
        return
    fi
    if command -v wget >/dev/null 2>&1; then
        wget -qO "$output" "$url"
        return
    fi
    log "curl or wget is required to install a release binary"
    return 1
}

release_download_url() {
    local target=$1
    local asset="rustyred-$target.tar.gz"
    if [[ -n "${THEOREM_RELEASE_VERSION:-}" ]]; then
        printf 'https://github.com/%s/releases/download/%s/%s\n' \
            "$GITHUB_REPO" "$THEOREM_RELEASE_VERSION" "$asset"
    else
        printf 'https://github.com/%s/releases/latest/download/%s\n' "$GITHUB_REPO" "$asset"
    fi
}

install_release_binary() {
    local target url tmp archive
    target="$(detect_release_target)" || return 1
    url="$(release_download_url "$target")"
    tmp="$(mktemp -d)"
    archive="$tmp/rustyred.tar.gz"
    log "downloading $url"
    if ! download_file "$url" "$archive"; then
        rm -rf "$tmp"
        return 1
    fi
    tar -xzf "$archive" -C "$tmp"
    if [[ ! -x "$tmp/theorem-agentd" ]]; then
        log "release archive did not contain theorem-agentd"
        rm -rf "$tmp"
        return 1
    fi
    mkdir -p "$INSTALL_DIR"
    install -m 0755 "$tmp/theorem-agentd" "$INSTALL_DIR/theorem-agentd"
    if [[ -x "$tmp/theorem-localmodel" ]]; then
        install -m 0755 "$tmp/theorem-localmodel" "$INSTALL_DIR/theorem-localmodel"
    else
        ln -sf "$INSTALL_DIR/theorem-agentd" "$INSTALL_DIR/theorem-localmodel"
    fi
    if [[ -x "$tmp/rustyred-proxy" ]]; then
        install -m 0755 "$tmp/rustyred-proxy" "$INSTALL_DIR/rustyred-proxy"
    fi
    if [[ -f "$tmp/theorem" ]]; then
        install -m 0755 "$tmp/theorem" "$INSTALL_DIR/theorem"
    else
        log "release archive did not contain theorem wrapper"
        rm -rf "$tmp"
        return 1
    fi
    if [[ -f "$tmp/rustyred" ]]; then
        install -m 0755 "$tmp/rustyred" "$INSTALL_DIR/rustyred"
    else
        ln -sf "$INSTALL_DIR/theorem" "$INSTALL_DIR/rustyred"
    fi
    rm -rf "$tmp"
    log "installed $INSTALL_DIR/theorem-agentd"
    log "installed $INSTALL_DIR/theorem-localmodel"
    if [[ -x "$INSTALL_DIR/rustyred-proxy" ]]; then
        log "installed $INSTALL_DIR/rustyred-proxy"
    fi
    log "installed $INSTALL_DIR/theorem"
    log "installed $INSTALL_DIR/rustyred"
}

local_checkout() {
    local script_repo=""
    if [[ -n "$SCRIPT_DIR" ]]; then
        script_repo="$(cd "$SCRIPT_DIR/.." 2>/dev/null && pwd)" || script_repo=""
    fi
    if [[ -n "$script_repo" && -f "$script_repo/rustyredcore_THG/Cargo.toml" ]]; then
        printf '%s\n' "$script_repo"
        return 0
    fi
    return 1
}

resolve_source_checkout() {
    if [[ -n "${THEOREM_SOURCE_DIR:-}" ]]; then
        if [[ ! -d "$SOURCE_DIR/.git" ]]; then
            require_command git
            mkdir -p "$(dirname "$SOURCE_DIR")"
            git clone "$REPO_URL" "$SOURCE_DIR"
        fi
        printf '%s\n' "$SOURCE_DIR"
        return 0
    fi

    if local_checkout; then
        return 0
    fi

    require_command git
    mkdir -p "$(dirname "$SOURCE_DIR")"
    if [[ -d "$SOURCE_DIR/.git" ]]; then
        log "updating $SOURCE_DIR"
        git -C "$SOURCE_DIR" pull --ff-only
    else
        log "cloning $REPO_URL into $SOURCE_DIR"
        git clone --depth 1 "$REPO_URL" "$SOURCE_DIR"
    fi
    printf '%s\n' "$SOURCE_DIR"
}

agentd_already_running() {
    if [[ ! -f "$PID_FILE" ]]; then
        return 1
    fi
    local pid
    pid="$(cat "$PID_FILE")"
    kill -0 "$pid" 2>/dev/null
}

start_agentd() {
    local source_checkout=${1:-}
    if [[ "${THEOREM_INSTALL_SKIP_START:-0}" == "1" ]]; then
        log "skipping agentd start because THEOREM_INSTALL_SKIP_START=1"
        return 0
    fi
    if [[ ! -x "$INSTALL_DIR/theorem-localmodel" && ! -x "$INSTALL_DIR/theorem-agentd" ]] && ! command -v cargo >/dev/null 2>&1; then
        log "cargo not found and no theorem-localmodel binary installed; did not start daemon"
        return 0
    fi
    mkdir -p "$THEOREM_HOME_DIR"
    if agentd_already_running; then
        log "agentd already running with pid $(cat "$PID_FILE")"
        return 0
    fi

    log "starting theorem-localmodel in the background"
    if [[ -n "$source_checkout" ]]; then
        THEOREM_REPO="$source_checkout" nohup "$INSTALL_DIR/theorem" start \
            >"$OUT_LOG" 2>"$ERR_LOG" </dev/null &
    else
        nohup "$INSTALL_DIR/theorem" start >"$OUT_LOG" 2>"$ERR_LOG" </dev/null &
    fi
    printf '%s\n' "$!" >"$PID_FILE"
    log "agentd pid $(cat "$PID_FILE")"
    log "logs: theorem logs"
}

main() {
    if [[ "${THEOREM_INSTALL_FROM_SOURCE:-0}" != "1" ]] && ! local_checkout >/dev/null; then
        if install_release_binary; then
            if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
                log "add $INSTALL_DIR to PATH to run rustyred from any shell"
            fi
            start_agentd
            log "proxy: rustyred proxy or rustyred wrap claude"
            return 0
        fi
        log "release install unavailable; falling back to source checkout"
    fi

    local source_checkout
    source_checkout="$(resolve_source_checkout)"

    mkdir -p "$INSTALL_DIR"
    install -m 0755 "$source_checkout/scripts/theorem" "$INSTALL_DIR/theorem"
    log "installed $INSTALL_DIR/theorem"
    if [[ -x "$source_checkout/rustyredcore_THG/target/release/rustyred-proxy" ]]; then
        install -m 0755 "$source_checkout/rustyredcore_THG/target/release/rustyred-proxy" "$INSTALL_DIR/rustyred-proxy"
        log "installed $INSTALL_DIR/rustyred-proxy"
    fi
    ln -sf "$INSTALL_DIR/theorem" "$INSTALL_DIR/rustyred"
    log "installed $INSTALL_DIR/rustyred"

    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        log "add $INSTALL_DIR to PATH to run theorem from any shell"
    fi

    start_agentd "$source_checkout"
    log "try: theorem init && theorem once \"hello from Theorem\""
    log "proxy: rustyred proxy or rustyred wrap claude"
}

main "$@"
