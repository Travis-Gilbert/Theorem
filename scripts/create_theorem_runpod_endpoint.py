#!/usr/bin/env python3
"""Create a RunPod Serverless endpoint for Theorem RustyRed training."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


REST_BASE = 'https://rest.runpod.io/v1'


def main() -> int:
    parser = argparse.ArgumentParser(
        description='Create a RunPod template and serverless endpoint.',
    )
    parser.add_argument('--env-file', help='Optional .env file with RUNPOD_API_KEY.')
    parser.add_argument('--rest-base', default=REST_BASE)
    parser.add_argument(
        '--image',
        default='ghcr.io/travis-gilbert/theorem-rustyred-training-worker:latest',
    )
    parser.add_argument('--template-name', default='theorem-rustyred-training-worker')
    parser.add_argument('--endpoint-name', default='theorem-rustyred-training')
    parser.add_argument('--workers-min', type=int, default=0)
    parser.add_argument('--workers-max', type=int, default=1)
    parser.add_argument('--gpu-type-id', action='append')
    parser.add_argument('--gpu-count', type=int, default=1)
    parser.add_argument('--container-disk-gb', type=int, default=20)
    parser.add_argument(
        '--container-registry-auth-id',
        help='Optional RunPod registry auth id for private images.',
    )
    parser.add_argument('--execution-timeout-ms', type=int, default=3_600_000)
    parser.add_argument('--create', action='store_true', help='Actually call RunPod APIs.')
    args = parser.parse_args()

    env = dict(os.environ)
    if args.env_file:
        env.update(read_env_file(Path(args.env_file)))
    api_key = env.get('RUNPOD_API_KEY', '').strip()
    if not api_key:
        print('RUNPOD_API_KEY is required.', file=sys.stderr)
        return 2

    template_body = {
        'name': args.template_name,
        'imageName': args.image,
        'category': 'NVIDIA',
        'containerDiskInGb': args.container_disk_gb,
        'dockerEntrypoint': [],
        'dockerStartCmd': [],
        'env': worker_env_from(env),
        'isPublic': False,
        'isServerless': True,
        'ports': [],
        'readme': 'Theorem RustyRed training worker.',
    }
    if args.container_registry_auth_id:
        template_body['containerRegistryAuthId'] = args.container_registry_auth_id
    endpoint_body = {
        'name': args.endpoint_name,
        'workersMin': args.workers_min,
        'workersMax': args.workers_max,
        'gpuCount': args.gpu_count,
        'executionTimeoutMs': args.execution_timeout_ms,
        'scalerType': 'QUEUE_DELAY',
        'scalerValue': 5,
        'computeType': 'GPU',
        'gpuTypeIds': args.gpu_type_id or ['NVIDIA L40S'],
    }

    if not args.create:
        print(json.dumps({
            'dry_run': True,
            'template_body': redact(template_body),
            'endpoint_body': endpoint_body,
        }, indent=2, sort_keys=True))
        return 0

    template = post_json(args.rest_base, '/templates', api_key, template_body)
    template_id = str(template.get('id') or template.get('templateId') or '')
    if not template_id:
        print(json.dumps({'template_response': redact(template)}, indent=2), file=sys.stderr)
        return 1
    endpoint = post_json(
        args.rest_base,
        '/endpoints/serverless',
        api_key,
        {**endpoint_body, 'templateId': template_id},
    )
    print(json.dumps({
        'ok': True,
        'template_id': template_id,
        'endpoint_id': endpoint.get('id') or endpoint.get('endpointId'),
        'template': redact(template),
        'endpoint': redact(endpoint),
    }, indent=2, sort_keys=True))
    return 0


def worker_env_from(env: dict[str, str]) -> dict[str, str]:
    keys = [
        'AWS_ACCESS_KEY_ID',
        'AWS_SECRET_ACCESS_KEY',
        'AWS_STORAGE_BUCKET_NAME',
        'AWS_S3_ENDPOINT_URL',
        'AWS_S3_REGION_NAME',
        'THEOREM_TRAINING_OUTPUT_PREFIX',
    ]
    return {key: env[key] for key in keys if env.get(key)}


def post_json(base: str, path: str, api_key: str, body: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        f"{base.rstrip('/')}{path}",
        data=json.dumps(body).encode('utf-8'),
        headers={
            'Authorization': f'Bearer {api_key}',
            'Content-Type': 'application/json',
        },
        method='POST',
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.loads(response.read().decode('utf-8'))
    except urllib.error.HTTPError as exc:
        raise SystemExit(json.dumps({
            'ok': False,
            'status': exc.code,
            'body': exc.read().decode('utf-8', errors='replace'),
        }, indent=2))


def read_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith('#') or '=' not in stripped:
            continue
        key, value = stripped.split('=', 1)
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def redact(value: Any) -> Any:
    if isinstance(value, dict):
        redacted = {}
        for key, item in value.items():
            upper = key.upper()
            if any(marker in upper for marker in ['KEY', 'SECRET', 'TOKEN', 'PASSWORD']):
                redacted[key] = '<redacted>'
            else:
                redacted[key] = redact(item)
        return redacted
    if isinstance(value, list):
        return [redact(item) for item in value]
    return value


if __name__ == '__main__':
    raise SystemExit(main())
