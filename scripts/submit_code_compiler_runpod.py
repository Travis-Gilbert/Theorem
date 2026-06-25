#!/usr/bin/env python3
"""Submit a Theorem code compiler burst request to a RunPod endpoint."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


DEFAULT_API_BASE = "https://api.runpod.ai/v2"
TERMINAL_STATUSES = {"COMPLETED", "FAILED", "CANCELLED", "TIMED_OUT"}


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Submit a CodeRunPodBurstRequest to RunPod.",
    )
    parser.add_argument("--input", required=True, help="Path to request JSON.")
    parser.add_argument("--endpoint-id", help="RunPod endpoint id.")
    parser.add_argument("--env-file", help="Optional .env file to read.")
    parser.add_argument("--api-base", default=DEFAULT_API_BASE)
    parser.add_argument("--sync", action="store_true", help="Use runsync instead of run.")
    parser.add_argument("--poll", action="store_true", help="Poll async jobs until terminal.")
    parser.add_argument("--poll-interval", type=float, default=5.0)
    parser.add_argument("--timeout-seconds", type=float, default=900.0)
    parser.add_argument("--output", help="Write the full RunPod response JSON.")
    parser.add_argument(
        "--worker-output",
        help="Write only the worker output object, suitable for Rust import smoke.",
    )
    args = parser.parse_args()

    env = dict(os.environ)
    if args.env_file:
        env.update(read_env_file(Path(args.env_file)))

    api_key = env.get("RUNPOD_API_KEY", "").strip()
    if not api_key:
        print("RUNPOD_API_KEY is required.", file=sys.stderr)
        return 2
    endpoint_id = (
        args.endpoint_id
        or env.get("RUNPOD_THEOREM_CODE_COMPILER_ENDPOINT_ID")
        or env.get("RUNPOD_CODE_COMPILER_ENDPOINT_ID")
        or env.get("RUNPOD_ENDPOINT_ID")
        or ""
    ).strip()
    if not endpoint_id:
        print(
            "RunPod endpoint id is required. Set RUNPOD_THEOREM_CODE_COMPILER_ENDPOINT_ID "
            "or pass --endpoint-id.",
            file=sys.stderr,
        )
        return 2

    payload = json.loads(Path(args.input).read_text())
    operation = "runsync" if args.sync else "run"
    response = post_json(
        f"{args.api_base.rstrip('/')}/{endpoint_id}/{operation}",
        api_key,
        {"input": payload},
    )
    if args.poll and not args.sync:
        job_id = str(response.get("id") or "")
        if not job_id:
            print(json.dumps({"ok": False, "response": response}, indent=2), file=sys.stderr)
            return 1
        response = poll_status(
            args.api_base.rstrip("/"),
            endpoint_id,
            job_id,
            api_key,
            args.poll_interval,
            args.timeout_seconds,
        )

    output = response.get("output")
    if args.output:
        Path(args.output).write_text(json.dumps(response, indent=2, sort_keys=True))
    if args.worker_output and output is not None:
        Path(args.worker_output).write_text(json.dumps(output, indent=2, sort_keys=True))
    print(json.dumps({"ok": True, "endpoint_id": endpoint_id, "response": response}, indent=2))
    return 0 if not is_failure(response) else 1


def post_json(url: str, api_key: str, body: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    return open_json(request)


def poll_status(
    api_base: str,
    endpoint_id: str,
    job_id: str,
    api_key: str,
    poll_interval: float,
    timeout_seconds: float,
) -> dict[str, Any]:
    deadline = time.time() + timeout_seconds
    status_url = f"{api_base}/{endpoint_id}/status/{job_id}"
    latest: dict[str, Any] = {}
    while time.time() < deadline:
        request = urllib.request.Request(
            status_url,
            headers={"Authorization": f"Bearer {api_key}"},
            method="GET",
        )
        latest = open_json(request)
        status = str(latest.get("status") or "").upper()
        if status in TERMINAL_STATUSES:
            return latest
        time.sleep(max(0.25, poll_interval))
    raise SystemExit(
        json.dumps(
            {"ok": False, "error": "runpod_poll_timeout", "latest": latest},
            indent=2,
        )
    )


def open_json(request: urllib.request.Request) -> dict[str, Any]:
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:
        raise SystemExit(
            json.dumps(
                {
                    "ok": False,
                    "status": exc.code,
                    "body": exc.read().decode("utf-8", errors="replace"),
                },
                indent=2,
            )
        )
    try:
        return json.loads(body)
    except json.JSONDecodeError:
        return {"raw": body}


def is_failure(response: dict[str, Any]) -> bool:
    status = str(response.get("status") or "").upper()
    output = response.get("output")
    return status in {"FAILED", "CANCELLED", "TIMED_OUT"} or (
        isinstance(output, dict) and bool(output.get("error"))
    )


def read_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


if __name__ == "__main__":
    raise SystemExit(main())
