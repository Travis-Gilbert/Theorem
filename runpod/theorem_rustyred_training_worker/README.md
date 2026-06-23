# Theorem RustyRed Training Worker

RunPod Serverless worker for Theorem-native RustyRed training snapshots.

The worker consumes the `runpod_input.json` emitted by:

```bash
cargo run -p rustyred-thg-adapters --bin theorem_training_run -- export \
  --data-dir /data/rustyred-training \
  --output-dir /tmp/theorem-training-export \
  --tenant theorem \
  --export-id theorem-training-001
```

Before submitting to RunPod, upload `manifest.json` and `graph_snapshot.json`
to remote object storage and pass their `s3://` URIs through
`scripts/submit_theorem_training_runpod.py`.

The current worker trains a dependency-light link-prediction embedding model
over the snapshot edges and returns a `ModelArtifactInput` object. The local
Theorem operator writes that object back into RedCore with:

```bash
cargo run -p rustyred-thg-adapters --bin theorem_training_run -- writeback \
  --data-dir /data/rustyred-training \
  --input model_artifact.json \
  --actor runpod
```

## Local Smoke

```bash
THEOREM_TRAINING_ALLOW_UNUPLOADED_ARTIFACT=true \
python3 runpod/theorem_rustyred_training_worker/worker.py \
  --local-input /tmp/theorem-rustyred-training-export/runpod_input.json
```

## Image

```bash
docker build \
  -t ghcr.io/travis-gilbert/theorem-rustyred-training-worker:latest \
  runpod/theorem_rustyred_training_worker
```

The container entrypoint is `python -u /app/worker.py`, matching the current
RunPod Serverless SDK `runpod.serverless.start({"handler": handler})` shape.
