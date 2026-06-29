#!/usr/bin/env python3
"""Seed the local Theorem node with the memory corpus (roadmap G1).

POSTs each `memory/*.md` as an `encode` to the node's MCP endpoint so the proxy's
ambient injection -- and `recall`/`hippo_retrieve` -- return real memories instead of an
empty store. Stdlib only.

Env: THEOREM_NODE_URL (default http://127.0.0.1:8380/mcp),
     THEOREM_MEMORY_DIR (default the CC project memory dir),
     THEOREM_TENANT (default "default"),
     THEOREM_HIPPO_SEED_INDEX_LIMIT (optional warmup page limit).
"""
import json
import os
import urllib.request

NODE = os.environ.get("THEOREM_NODE_URL", "http://127.0.0.1:8380/mcp")
PROJECT_KEY = os.getcwd().replace(os.sep, "-")
MEM_DIR = os.environ.get(
    "THEOREM_MEMORY_DIR",
    os.path.expanduser(f"~/.claude/projects/{PROJECT_KEY}/memory"),
)
TENANT = os.environ.get("THEOREM_TENANT", "default")


def call(name, arguments, timeout=20):
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": "seed",
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
    ).encode()
    req = urllib.request.Request(
        NODE, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        payload = json.load(response)
    if "error" in payload:
        raise RuntimeError(payload["error"])
    result = payload.get("result")
    if isinstance(result, dict) and result.get("isError"):
        raise RuntimeError(result)
    return payload


def main():
    if not os.path.isdir(MEM_DIR):
        raise SystemExit(f"memory dir not found: {MEM_DIR}")
    files = sorted(f for f in os.listdir(MEM_DIR) if f.endswith(".md"))
    ok = 0
    for name in files:
        with open(os.path.join(MEM_DIR, name), encoding="utf-8") as file:
            content = file.read()
        title = name[:-3]
        try:
            resp = call(
                "encode",
                {
                    "tenant": TENANT,
                    "actor": "memory-seed",
                    # The node accepts kind in {encode, feedback, solution, postmortem};
                    # "encode" is the generic catch-all for corpus seeding.
                    "kind": "encode",
                    "title": title,
                    "tags": ["memory-corpus"],
                    "content": content,
                },
            )
            good = "result" in resp
            print(f"{'ok ' if good else 'ERR'} {title}")
            ok += 1 if good else 0
        except Exception as error:  # noqa: BLE001 - report and continue
            print(f"ERR {title}: {error}")
    print(f"seeded {ok}/{len(files)} memories into {NODE}")
    # Warm the HippoRAG index once so the proxy's per-turn retrieval stays fast (the read
    # path never has to index). The first warm over a large corpus is slow (minutes);
    # later runs are quick once the index is built.
    try:
        warm_args = {
            "tenant": TENANT,
            "query": "warm the index",
            "top_k": 1,
            "auto_index_memory": True,
        }
        seed_index_limit = os.environ.get("THEOREM_HIPPO_SEED_INDEX_LIMIT")
        if seed_index_limit:
            warm_args["index_limit"] = int(seed_index_limit)
        call("hippo_retrieve", warm_args, timeout=600)
        print("index warmed")
    except Exception as error:  # noqa: BLE001
        print(f"warm step (first run is slow over a large corpus): {error}")


if __name__ == "__main__":
    main()
