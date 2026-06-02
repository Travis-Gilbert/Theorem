#!/usr/bin/env python3
"""Submit a Theorem RustyRed training snapshot to a RunPod endpoint.

The Rust CLI writes local snapshot files. A remote RunPod worker cannot read
those local paths, so this submitter requires remote manifest/snapshot URIs by
default and injects them into the request payload.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


DEFAULT_API_BASE = 'https://api.runpod.ai/v2'


def main() -> int:
    parser = argparse.ArgumentParser(
        description='Submit theorem_training_run runpod_input.json to RunPod.',
    )
    parser.add_argument('--input', required=True, help='Path to runpod_input.json.')
    parser.add_argument('--endpoint-id', help='RunPod endpoint id.')
    parser.add_argument('--env-file', help='Optional .env file to read.')
    parser.add_argument('--api-base', default=DEFAULT_API_BASE)
    parser.add_argument('--sync', action='store_true', help='Use runsync instead of run.')
    parser.add_argument('--manifest-uri', help='Remote URI for manifest.json.')
    parser.add_argument('--snapshot-uri', help='Remote URI for graph_snapshot.json.')
    parser.add_argument(
        '--allow-local-paths',
        action='store_true',
        help='Allow submitting local file paths. Use only for a worker on this host.',
    )
    args = parser.parse_args()

    env = dict(os.environ)
    if args.env_file:
        env.update(read_env_file(Path(args.env_file)))

    api_key = env.get('RUNPOD_API_KEY', '').strip()
    if not api_key:
        print('RUNPOD_API_KEY is required.', file=sys.stderr)
        return 2

    endpoint_id = (
        args.endpoint_id
        or env.get('RUNPOD_THEOREM_TRAINING_ENDPOINT_ID')
        or env.get('RUNPOD_RUSTYRED_TRAINING_ENDPOINT_ID')
        or env.get('RUNPOD_ENDPOINT_ID')
        or ''
    ).strip()
    if not endpoint_id:
        print(
            'RunPod endpoint id is required. Set RUNPOD_THEOREM_TRAINING_ENDPOINT_ID '
            'or pass --endpoint-id.',
            file=sys.stderr,
        )
        return 2

    payload = json.loads(Path(args.input).read_text())
    remote_files = dict(payload.get('remote_files') or {})
    if args.manifest_uri:
        remote_files['manifest_json'] = args.manifest_uri
    if args.snapshot_uri:
        remote_files['graph_snapshot_json'] = args.snapshot_uri

    if remote_files.get('manifest_json') and remote_files.get('graph_snapshot_json'):
        payload['remote_files'] = remote_files
    elif not args.allow_local_paths:
        print(
            'Remote manifest/snapshot URIs are required for RunPod submission. '
            'Upload the files and pass --manifest-uri plus --snapshot-uri, or use '
            '--allow-local-paths only for a worker on this host.',
            file=sys.stderr,
        )
        return 2

    operation = 'runsync' if args.sync else 'run'
    url = f"{args.api_base.rstrip('/')}/{endpoint_id}/{operation}"
    body = json.dumps({'input': payload}).encode('utf-8')
    request = urllib.request.Request(
        url,
        data=body,
        headers={
            'Authorization': f'Bearer {api_key}',
            'Content-Type': 'application/json',
        },
        method='POST',
    )

    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            response_body = response.read().decode('utf-8')
    except urllib.error.HTTPError as exc:
        print(
            json.dumps(
                {
                    'ok': False,
                    'status': exc.code,
                    'body': exc.read().decode('utf-8', errors='replace'),
                },
                indent=2,
            ),
            file=sys.stderr,
        )
        return 1

    parsed: Any
    try:
        parsed = json.loads(response_body)
    except json.JSONDecodeError:
        parsed = {'raw': response_body}
    print(json.dumps({'ok': True, 'endpoint_id': endpoint_id, 'response': parsed}, indent=2))
    return 0


def read_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith('#') or '=' not in stripped:
            continue
        key, value = stripped.split('=', 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


if __name__ == '__main__':
    raise SystemExit(main())
