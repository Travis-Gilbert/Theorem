#!/usr/bin/env python3
"""Upload Theorem RustyRed training snapshot files for RunPod."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any


def main() -> int:
    parser = argparse.ArgumentParser(
        description='Upload theorem_training_run export files to S3-compatible storage.',
    )
    parser.add_argument('--export-dir', required=True)
    parser.add_argument('--env-file', help='Optional .env file to read.')
    parser.add_argument(
        '--prefix',
        default='theorem-rustyred-training/snapshots',
        help='S3 key prefix for uploaded snapshot files.',
    )
    parser.add_argument(
        '--remote-input',
        default='runpod_input.remote.json',
        help='Filename to write inside --export-dir.',
    )
    args = parser.parse_args()

    env = dict(os.environ)
    if args.env_file:
        env.update(read_env_file(Path(args.env_file)))

    bucket = env.get('AWS_STORAGE_BUCKET_NAME', '').strip()
    if not bucket:
        print('AWS_STORAGE_BUCKET_NAME is required.', file=sys.stderr)
        return 2

    export_dir = Path(args.export_dir)
    manifest_path = export_dir / 'manifest.json'
    snapshot_path = export_dir / 'graph_snapshot.json'
    input_path = export_dir / 'runpod_input.json'
    for path in [manifest_path, snapshot_path, input_path]:
        if not path.exists():
            print(f'missing required file: {path}', file=sys.stderr)
            return 2

    try:
        import boto3
    except ImportError:
        print('boto3 is required. Install boto3 or run from an environment that has it.', file=sys.stderr)
        return 2

    client = boto3.client(
        's3',
        aws_access_key_id=env.get('AWS_ACCESS_KEY_ID') or None,
        aws_secret_access_key=env.get('AWS_SECRET_ACCESS_KEY') or None,
        endpoint_url=env.get('AWS_S3_ENDPOINT_URL') or None,
        region_name=env.get('AWS_S3_REGION_NAME') or None,
    )

    runpod_input = json.loads(input_path.read_text())
    snapshot_hash = str(runpod_input.get('snapshot_hash') or 'snapshot').replace('sha256:', '')
    prefix = f"{args.prefix.strip('/')}/{snapshot_hash}"
    manifest_key = f'{prefix}/manifest.json'
    snapshot_key = f'{prefix}/graph_snapshot.json'
    upload_file(client, bucket, manifest_key, manifest_path)
    upload_file(client, bucket, snapshot_key, snapshot_path)

    remote_input = {
        **runpod_input,
        'remote_files': {
            'manifest_json': f's3://{bucket}/{manifest_key}',
            'graph_snapshot_json': f's3://{bucket}/{snapshot_key}',
        },
    }
    remote_input_path = export_dir / args.remote_input
    remote_input_path.write_text(json.dumps(remote_input, indent=2, sort_keys=True))
    print(json.dumps({
        'ok': True,
        'bucket': bucket,
        'manifest_uri': remote_input['remote_files']['manifest_json'],
        'snapshot_uri': remote_input['remote_files']['graph_snapshot_json'],
        'remote_input_path': str(remote_input_path),
    }, indent=2))
    return 0


def upload_file(client: Any, bucket: str, key: str, path: Path) -> None:
    content_type = 'application/json'
    client.upload_file(
        str(path),
        bucket,
        key,
        ExtraArgs={'ContentType': content_type},
    )


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
