#!/usr/bin/env bash
# Install or remove Theorem Carry local lifecycle hooks for Claude Code and Codex.

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

usage() {
    cat <<'EOF'
usage: install.sh <install|uninstall|status>

Environment:
  THEOREM_REPO           Source checkout to install from.
  CLAUDE_SETTINGS_PATH   Override Claude settings path.
  CODEX_HOOKS_PATH       Override Codex hooks path.
EOF
}

main() {
    local command=${1:-install}
    case "$command" in
        install | uninstall | status)
            manage_hooks "$command"
            ;;
        -h | --help | help)
            usage
            ;;
        *)
            usage >&2
            return 1
            ;;
    esac
}

manage_hooks() {
    local command=$1
    local repo claude_settings codex_hooks
    repo="${THEOREM_REPO:-$REPO_ROOT}"
    claude_settings="${CLAUDE_SETTINGS_PATH:-$HOME/.claude/settings.json}"
    codex_hooks="${CODEX_HOOKS_PATH:-$HOME/.codex/hooks.json}"
    python3 - "$command" "$repo" "$claude_settings" "$codex_hooks" <<'PY'
import json
import shlex
import sys
import time
from pathlib import Path

command, repo, claude_settings, codex_hooks = sys.argv[1:5]
repo_path = Path(repo).resolve()
session_start = repo_path / "rustyredcore_THG" / "hooks" / "session_start.sh"
capture = repo_path / "rustyredcore_THG" / "hooks" / "carry-capture.sh"
marker = "THEOREM_CARRY_MANAGED=1"

def load_json(path, default):
    path = Path(path)
    if not path.exists():
        return default
    return json.loads(path.read_text())

def backup(path):
    path = Path(path)
    if not path.exists():
        return
    stamp = int(time.time())
    backup_path = path.with_suffix(path.suffix + f".theorem-carry.{stamp}.bak")
    backup_path.write_text(path.read_text())

def managed_command(agent, script, event=None):
    env = {
        "THEOREM_CARRY_MANAGED": "1",
        "THEOREM_CARRY_ENABLED": "1",
        "THEOREM_CARRY_AGENT": agent,
        "THEOREM_REPO": str(repo_path),
    }
    pieces = [f"{key}={shlex.quote(value)}" for key, value in env.items()]
    pieces.extend(["/bin/bash", shlex.quote(str(script))])
    if event:
        pieces.append(shlex.quote(event))
    return " ".join(pieces)

def session_entry(agent):
    return {
        "matcher": "startup|clear|resume",
        "hooks": [
            {
                "type": "command",
                "command": managed_command(agent, session_start),
                "timeout": 12,
                "statusMessage": "Loading Theorem Carry memory",
            }
        ],
    }

def capture_entry(agent, event, matcher=None):
    entry = {
        "hooks": [
            {
                "type": "command",
                "command": managed_command(agent, capture, event),
                "timeout": 8,
                "statusMessage": "Capturing Theorem Carry activity",
            }
        ]
    }
    if matcher:
        entry["matcher"] = matcher
    return entry

def remove_managed(data):
    hooks = data.setdefault("hooks", {})
    for event, entries in list(hooks.items()):
        kept = []
        for entry in entries:
            nested = entry.get("hooks", []) if isinstance(entry, dict) else []
            has_marker = any(marker in str(hook.get("command", "")) for hook in nested if isinstance(hook, dict))
            if not has_marker:
                kept.append(entry)
        if kept:
            hooks[event] = kept
        else:
            hooks.pop(event, None)
    return data

def install_entries(data, agent):
    hooks = data.setdefault("hooks", {})
    remove_managed(data)
    hooks.setdefault("SessionStart", []).append(session_entry(agent))
    hooks.setdefault("UserPromptSubmit", []).append(capture_entry(agent, "UserPromptSubmit"))
    matcher = "Bash|Edit|Write|NotebookEdit|exec_command|functions.exec_command|apply_patch|functions.apply_patch"
    hooks.setdefault("PostToolUse", []).append(capture_entry(agent, "PostToolUse", matcher))
    hooks.setdefault("Stop", []).append(capture_entry(agent, "Stop"))
    return data

def count_managed(data):
    count = 0
    for entries in data.get("hooks", {}).values():
        for entry in entries:
            for hook in entry.get("hooks", []):
                if marker in str(hook.get("command", "")):
                    count += 1
    return count

if command == "status":
    claude_data = load_json(claude_settings, {})
    codex_data = load_json(codex_hooks, {})
    print(json.dumps({
        "claude_settings": claude_settings,
        "claude_managed_hooks": count_managed(claude_data),
        "codex_hooks": codex_hooks,
        "codex_managed_hooks": count_managed(codex_data),
    }, indent=2))
    sys.exit(0)

for path_text, agent, default in [
    (claude_settings, "claude-code", {}),
    (codex_hooks, "codex", {"hooks": {}}),
]:
    path = Path(path_text)
    path.parent.mkdir(parents=True, exist_ok=True)
    data = load_json(path, default)
    backup(path)
    if command == "install":
        data = install_entries(data, agent)
    elif command == "uninstall":
        data = remove_managed(data)
    path.write_text(json.dumps(data, indent=2, sort_keys=False) + "\n")

print(f"theorem-carry {command} complete")
PY
}

main "$@"
