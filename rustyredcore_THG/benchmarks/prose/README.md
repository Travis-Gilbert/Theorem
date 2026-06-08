# Writing-Engineering Prose Benchmark

This benchmark follows the caveman benchmark shape: prompts x modes x trials,
median output tokens, deterministic temperature 0, and a source hash pinned
into the results. It adds two writing-engineering columns: fidelity preservation
and clutter score for the normal baseline.

Run:

```bash
python3 benchmarks/prose/run.py --trials 3
```

Modes:

- `normal`
- `plain`
- `spare`
- `wire`
- `caveman-full`

The current harness is offline-fixture mode. It validates the measurement
contract and gate math without requiring provider API keys. Live model calls can
replace `response_for()` while preserving the result schema.
