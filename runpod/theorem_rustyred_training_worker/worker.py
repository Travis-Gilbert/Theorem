#!/usr/bin/env python3
"""RunPod Serverless worker for Theorem RustyRed training snapshots."""

from __future__ import annotations

import hashlib
import json
import math
import os
import random
import sys
import time
import urllib.request
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


def handler(job: dict[str, Any]) -> dict[str, Any]:
    """Run a small graph-embedding training job over a RustyRed snapshot."""
    started = time.time()
    job_input = job.get('input') or {}
    if not isinstance(job_input, dict):
        return {'error': 'job input must be an object'}

    try:
        manifest, snapshot = load_training_payload(job_input)
        tenant_id = str(job_input.get('tenant_id') or manifest['tenant_id'])
        model_id = str(
            job_input.get('model_id')
            or f"rustyred-graph-embedding-{manifest['snapshot_hash'][-12:]}"
        )
        model_type = str(job_input.get('model_type') or 'rustyred_graph_embedding')
        embedding_dim = int(job_input.get('embedding_dim') or 16)
        epochs = int(job_input.get('epochs') or 40)
        learning_rate = float(job_input.get('learning_rate') or 0.05)

        nodes, edges = selected_training_graph(snapshot, tenant_id)
        trained = train_link_prediction_embeddings(
            nodes=nodes,
            edges=edges,
            embedding_dim=embedding_dim,
            epochs=epochs,
            learning_rate=learning_rate,
            seed_text=manifest['snapshot_hash'],
        )
        artifact = {
            'artifact_kind': 'theorem_rustyred_graph_embedding_v1',
            'model_id': model_id,
            'tenant_id': tenant_id,
            'model_type': model_type,
            'source_graph_version': manifest['graph_version'],
            'dataset_hash': manifest['snapshot_hash'],
            'created_at_unix': int(time.time()),
            'training': trained,
        }
        s3_uri = upload_artifact_if_configured(job_input, model_id, artifact)
        if not s3_uri:
            if not truthy(job_input.get('allow_unuploaded_artifact')) and not truthy(
                os.environ.get('THEOREM_TRAINING_ALLOW_UNUPLOADED_ARTIFACT')
            ):
                return {
                    'error': (
                        'artifact upload is not configured. Set output_s3_uri or '
                        'THEOREM_TRAINING_OUTPUT_PREFIX with AWS credentials, or '
                        'allow_unuploaded_artifact=true for local smoke only.'
                    ),
                    'metrics': trained['metrics'],
                }
            s3_uri = f"s3://unuploaded-theorem-training/{model_id}/artifact.json"

        trained_on = trained_on_node_ids(manifest)
        model_artifact_input = {
            'model_id': model_id,
            'tenant_id': tenant_id,
            'model_type': model_type,
            's3_uri': s3_uri,
            'dataset_hash': manifest['snapshot_hash'],
            'source_graph_version': manifest['graph_version'],
            'trained_on_node_ids': trained_on,
            'metrics': {
                **trained['metrics'],
                'worker': 'theorem_rustyred_training_worker',
                'elapsed_seconds': round(time.time() - started, 3),
            },
            'promotion_decision': str(job_input.get('promotion_decision') or 'shadow'),
            'manifest_version': 1,
        }
        return {
            'ok': True,
            'model_artifact': model_artifact_input,
            'artifact': {
                's3_uri': s3_uri,
                'node_count': len(nodes),
                'edge_count': len(edges),
                'embedding_dim': embedding_dim,
            },
        }
    except Exception as exc:  # noqa: BLE001 - worker must return provider-readable errors.
        return {'error': str(exc), 'error_type': type(exc).__name__}


def load_training_payload(job_input: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    remote_files = job_input.get('remote_files') or {}
    local_files = job_input.get('local_files') or {}
    manifest_uri = remote_files.get('manifest_json') or local_files.get('manifest_json')
    snapshot_uri = remote_files.get('graph_snapshot_json') or local_files.get('graph_snapshot_json')
    if not manifest_uri or not snapshot_uri:
        raise ValueError('manifest_json and graph_snapshot_json are required')

    manifest = read_json_resource(str(manifest_uri))
    snapshot_bundle = read_json_resource(str(snapshot_uri))
    snapshot = snapshot_bundle.get('snapshot') or snapshot_bundle
    expected_hash = str(job_input.get('snapshot_hash') or manifest.get('snapshot_hash') or '')
    bundle_hash = str((snapshot_bundle.get('manifest') or {}).get('snapshot_hash') or '')
    if expected_hash and bundle_hash and expected_hash != bundle_hash:
        raise ValueError(
            f'snapshot hash mismatch: job={expected_hash} bundle={bundle_hash}'
        )
    for key in ['tenant_id', 'graph_version', 'snapshot_hash']:
        if key not in manifest:
            raise ValueError(f'manifest missing {key}')
    if 'nodes' not in snapshot or 'edges' not in snapshot:
        raise ValueError('graph snapshot must contain nodes and edges')
    return manifest, snapshot


def read_json_resource(uri: str) -> dict[str, Any]:
    parsed = urlparse(uri)
    if parsed.scheme in ('http', 'https'):
        with urllib.request.urlopen(uri, timeout=120) as response:
            return json.loads(response.read().decode('utf-8'))
    if parsed.scheme == 's3':
        return read_s3_json(parsed.netloc, parsed.path.lstrip('/'))
    path = Path(parsed.path if parsed.scheme == 'file' else uri)
    return json.loads(path.read_text())


def read_s3_json(bucket: str, key: str) -> dict[str, Any]:
    import boto3

    client = boto3.client(
        's3',
        endpoint_url=os.environ.get('AWS_S3_ENDPOINT_URL') or None,
        region_name=os.environ.get('AWS_S3_REGION_NAME') or None,
    )
    response = client.get_object(Bucket=bucket, Key=key)
    return json.loads(response['Body'].read().decode('utf-8'))


def selected_training_graph(
    snapshot: dict[str, Any],
    tenant_id: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    nodes = [
        node
        for node in snapshot.get('nodes', [])
        if not node.get('tombstone')
        and (node.get('properties') or {}).get('tenant_id') == tenant_id
    ]
    node_ids = {node['id'] for node in nodes}
    edges = [
        edge
        for edge in snapshot.get('edges', [])
        if not edge.get('tombstone')
        and edge.get('from_id') in node_ids
        and edge.get('to_id') in node_ids
    ]
    if not nodes:
        raise ValueError(f'no tenant-scoped nodes found for {tenant_id}')
    return nodes, edges


def train_link_prediction_embeddings(
    *,
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
    embedding_dim: int,
    epochs: int,
    learning_rate: float,
    seed_text: str,
) -> dict[str, Any]:
    node_ids = [node['id'] for node in nodes]
    index = {node_id: idx for idx, node_id in enumerate(node_ids)}
    edge_pairs = [
        (index[edge['from_id']], index[edge['to_id']])
        for edge in edges
        if edge.get('from_id') in index and edge.get('to_id') in index
    ]
    seed = int(hashlib.sha256(seed_text.encode('utf-8')).hexdigest()[:16], 16)
    rng = random.Random(seed)
    embeddings = [
        [rng.uniform(-0.05, 0.05) for _ in range(embedding_dim)] for _ in node_ids
    ]

    losses: list[float] = []
    if edge_pairs:
        for _epoch in range(max(1, epochs)):
            total_loss = 0.0
            updates = 0
            for src, dst in edge_pairs:
                neg = rng.randrange(len(node_ids))
                if neg == dst and len(node_ids) > 1:
                    neg = (neg + 1) % len(node_ids)
                pos_score = dot(embeddings[src], embeddings[dst])
                neg_score = dot(embeddings[src], embeddings[neg])
                loss = max(0.0, 1.0 - pos_score + neg_score)
                total_loss += loss
                if loss > 0.0:
                    src_vec = embeddings[src][:]
                    dst_vec = embeddings[dst][:]
                    neg_vec = embeddings[neg][:]
                    for dim in range(embedding_dim):
                        embeddings[src][dim] += learning_rate * (dst_vec[dim] - neg_vec[dim])
                        embeddings[dst][dim] += learning_rate * src_vec[dim]
                        embeddings[neg][dim] -= learning_rate * src_vec[dim]
                    updates += 1
            normalize_rows(embeddings)
            losses.append(total_loss / max(1, len(edge_pairs)))
            if updates == 0:
                break
    else:
        normalize_rows(embeddings)
        losses.append(0.0)

    return {
        'node_ids': node_ids,
        'embeddings': embeddings,
        'metrics': {
            'trainer': 'stdlib_margin_link_prediction',
            'epochs_requested': epochs,
            'epochs_actual': len(losses),
            'embedding_dim': embedding_dim,
            'nodes_total': len(node_ids),
            'edges_total': len(edge_pairs),
            'initial_loss': losses[0] if losses else 0.0,
            'final_loss': losses[-1] if losses else 0.0,
        },
    }


def upload_artifact_if_configured(
    job_input: dict[str, Any],
    model_id: str,
    artifact: dict[str, Any],
) -> str:
    configured_uri = str(job_input.get('output_s3_uri') or '').strip()
    bucket = os.environ.get('AWS_STORAGE_BUCKET_NAME', '').strip()
    prefix = str(
        job_input.get('output_prefix')
        or os.environ.get('THEOREM_TRAINING_OUTPUT_PREFIX')
        or 'theorem-rustyred-training/models'
    ).strip('/')
    if configured_uri:
        parsed = urlparse(configured_uri)
        if parsed.scheme != 's3':
            raise ValueError('output_s3_uri must be s3://')
        bucket = parsed.netloc
        key = parsed.path.lstrip('/')
        return upload_s3_json(bucket, key, artifact)
    if not bucket:
        return ''
    key = f'{prefix}/{model_id}/artifact.json'
    return upload_s3_json(bucket, key, artifact)


def upload_s3_json(bucket: str, key: str, value: dict[str, Any]) -> str:
    import boto3

    client = boto3.client(
        's3',
        endpoint_url=os.environ.get('AWS_S3_ENDPOINT_URL') or None,
        region_name=os.environ.get('AWS_S3_REGION_NAME') or None,
    )
    body = json.dumps(value, separators=(',', ':'), sort_keys=True).encode('utf-8')
    client.put_object(
        Bucket=bucket,
        Key=key,
        Body=body,
        ContentType='application/json',
    )
    return f's3://{bucket}/{key}'


def trained_on_node_ids(manifest: dict[str, Any]) -> list[str]:
    result: list[str] = []
    for key in [
        'gnn_export_ids',
        'paraphrase_pair_ids',
        'reasoning_trace_ids',
        'artifact_ids',
    ]:
        values = manifest.get(key) or []
        if isinstance(values, list):
            result.extend(str(value) for value in values if str(value).strip())
    return result


def dot(left: list[float], right: list[float]) -> float:
    return sum(a * b for a, b in zip(left, right))


def normalize_rows(rows: list[list[float]]) -> None:
    for row in rows:
        norm = math.sqrt(sum(value * value for value in row)) or 1.0
        for idx, value in enumerate(row):
            row[idx] = round(value / norm, 8)


def truthy(value: Any) -> bool:
    return str(value).strip().lower() in {'1', 'true', 'yes', 'on'}


def run_local(input_path: str) -> int:
    payload = json.loads(Path(input_path).read_text())
    result = handler({'input': payload})
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result.get('ok') else 1


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--local-input':
        raise SystemExit(run_local(sys.argv[2]))
    import runpod

    runpod.serverless.start({'handler': handler})
