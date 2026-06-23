#!/usr/bin/env bash
# SPEC-CONTEXT-MEMBRANE-1.0 code-arm reflex (automatic entry point).
#
# On session start: detect the repo root and current git SHA, ensure the repo's
# code knowledge graph is resident at that SHA (a snapshot LOAD when already
# ingested, an incremental reindex on a changed SHA, a full ingest only the first
# time), then run the membrane-gated `context_pack` for the task and print the
# gated code map to stdout so the harness injects it as session context. The
# agent starts oriented against warm, budget-gated structure.
#
# Fail-open by contract: a session hook runs every time, so every failure path
# (not a git repo, no tenant scope, server unreachable, missing python) degrades
# to silence and exits 0. It must never break or slow a session it cannot help.
#
# Wiring:
#   Claude Code: register as a SessionStart hook command.
#   Codex: register as the codex session-start equivalent (same script).
# Config (env):
#   THEOREM_TENANT_ID    tenant scope (required; no tenant => no injection)
#   THEOREM_MCP_URL      MCP JSON-RPC endpoint (default: the product server /mcp)
#   THEOREM_API_TOKEN    optional bearer token for the endpoint
#   THEOREM_CONTEXT_TASK optional task string the pack is conditioned on
#   THEOREM_CONTEXT_BUDGET_TOKENS  token budget for the pack (default 2000)

# Never let a failure escape: no `set -e`, and a final `exit 0` regardless.
_membrane_reflex() {
  command -v git >/dev/null 2>&1 || return 0
  command -v python3 >/dev/null 2>&1 || return 0

  local repo_root sha tenant mcp_url token task budget repo_url repo_id
  repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || return 0
  [ -n "$repo_root" ] || return 0
  sha=$(git -C "$repo_root" rev-parse HEAD 2>/dev/null) || return 0

  tenant=${THEOREM_TENANT_ID:-}
  [ -n "$tenant" ] || return 0  # no tenant scope => nothing to inject

  mcp_url=${THEOREM_MCP_URL:-https://rustyredcore-theorem-production.up.railway.app/mcp}
  token=${THEOREM_API_TOKEN:-}
  task=${THEOREM_CONTEXT_TASK:-}
  budget=${THEOREM_CONTEXT_BUDGET_TOKENS:-2000}

  # Repo identity: prefer the origin remote URL (drives ensure's clone/diff path),
  # fall back to the directory name as the repo id.
  repo_url=$(git -C "$repo_root" remote get-url origin 2>/dev/null || true)
  repo_id=$(basename "$repo_root")

  THEOREM_MCP_URL="$mcp_url" \
  THEOREM_API_TOKEN="$token" \
  THEOREM_TENANT_ID="$tenant" \
  MEMBRANE_REPO_URL="$repo_url" \
  MEMBRANE_REPO_ID="$repo_id" \
  MEMBRANE_SHA="$sha" \
  MEMBRANE_TASK="$task" \
  MEMBRANE_BUDGET="$budget" \
  python3 - <<'PY' 2>/dev/null || return 0
import json, os, sys, urllib.request

url = os.environ["THEOREM_MCP_URL"]
token = os.environ.get("THEOREM_API_TOKEN", "")
tenant = os.environ["THEOREM_TENANT_ID"]
repo_url = os.environ.get("MEMBRANE_REPO_URL", "")
repo_id = os.environ.get("MEMBRANE_REPO_ID", "")
sha = os.environ.get("MEMBRANE_SHA", "")
task = os.environ.get("MEMBRANE_TASK", "")
try:
    budget = int(os.environ.get("MEMBRANE_BUDGET", "2000"))
except ValueError:
    budget = 2000

def call(operation, args, timeout):
    payload = {
        "jsonrpc": "2.0",
        "id": operation,
        "method": "tools/call",
        "params": {
            "name": "compute_code",
            "arguments": dict(args, operation=operation, tenant=tenant, tenant_id=tenant),
        },
    }
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))

def result_payload(envelope):
    # tools/call result content is JSON text or a structured result; be liberal.
    if not isinstance(envelope, dict):
        return {}
    result = envelope.get("result", envelope)
    content = result.get("content") if isinstance(result, dict) else None
    if isinstance(content, list):
        for item in content:
            text = item.get("text") if isinstance(item, dict) else None
            if text:
                try:
                    return json.loads(text)
                except (ValueError, TypeError):
                    return {"code_map": text}
    return result if isinstance(result, dict) else {}

# 1. SHA-keyed ensure: snapshot load (cheap) / incremental reindex / full ingest.
#    Best-effort and only when we have a clone URL; ignore failure.
if repo_url:
    try:
        call("code_ingest_ensure", {"repo_url": repo_url, "sha": sha}, timeout=8)
    except Exception:
        pass

# 2. Membrane-gated context pack; inject its code map.
try:
    pack = result_payload(call(
        "context_pack",
        {"repo_id": repo_id, "repo_url": repo_url, "sha": sha, "task": task, "budget_tokens": budget},
        timeout=12,
    ))
except Exception:
    sys.exit(0)

code_map = pack.get("code_map") or ""
admitted = pack.get("admitted") or pack.get("admitted_context") or []
if not code_map and not admitted:
    sys.exit(0)

print("## Code neighborhood (membrane context_pack)")
print()
print(f"Repo `{repo_id}` at `{sha[:12]}` -- {len(admitted)} symbols admitted, "
      f"budget {budget} tokens (graph-aware reranked, deferred overflow recoverable via context_fetch).")
print()
if code_map:
    print(code_map)
PY
}

_membrane_reflex
exit 0
