# Theorem Code Compiler Worker

RunPod Serverless worker for Rust-native code compiler burst jobs.

The worker consumes the self-contained `CodeRunPodBurstRequest` emitted by
`rustyred-thg-code`. The request carries symbol snapshots and dependency edges,
so the worker does not need direct access to a local RedCore store.

Production behavior loads a learned `sentence-transformers` model and fails
closed if the model cannot be loaded. The explicit `allow_unlearned_fallback`
input flag exists only for local smoke tests.

## Local Smoke

```bash
python3 runpod/theorem_code_compiler_worker/worker.py \
  --local-input /tmp/code_compiler_runpod_request.json
```

## Image

```bash
docker build \
  -t ghcr.io/travis-gilbert/theorem-code-compiler-worker:latest \
  runpod/theorem_code_compiler_worker
```

The container entrypoint is `python -u /app/worker.py`, matching the RunPod
queue-based Serverless handler shape:

```python
runpod.serverless.start({"handler": handler})
```
