#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || true)"
readonly REPO_URL="${THEOREM_REPO_URL:-https://github.com/Travis-Gilbert/Theorem.git}"
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
    local source_checkout=$1
    if [[ "${THEOREM_INSTALL_SKIP_START:-0}" == "1" ]]; then
        log "skipping agentd start because THEOREM_INSTALL_SKIP_START=1"
        return 0
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        log "cargo not found; installed the CLI but did not start agentd"
        return 0
    fi
    mkdir -p "$THEOREM_HOME_DIR"
    if agentd_already_running; then
        log "agentd already running with pid $(cat "$PID_FILE")"
        return 0
    fi

    log "starting theorem-agentd in the background"
    THEOREM_REPO="$source_checkout" nohup "$INSTALL_DIR/theorem" start \
        >"$OUT_LOG" 2>"$ERR_LOG" </dev/null &
    printf '%s\n' "$!" >"$PID_FILE"
    log "agentd pid $(cat "$PID_FILE")"
    log "logs: theorem logs"
}

main() {
    local source_checkout
    source_checkout="$(resolve_source_checkout)"

    mkdir -p "$INSTALL_DIR"
    install -m 0755 "$source_checkout/scripts/theorem" "$INSTALL_DIR/theorem"
    log "installed $INSTALL_DIR/theorem"

    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        log "add $INSTALL_DIR to PATH to run theorem from any shell"
    fi

    start_agentd "$source_checkout"
    log "try: theorem init && theorem once \"hello from Theorem\""
}

main "$@"
