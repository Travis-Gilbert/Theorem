# theorem-prompt

Provider-neutral prompt assembly for Theorem head invocations. The crate turns a `HeadInvocationRequest` into a `PromptSpec`, separates cache-stable instruction blocks from dynamic task and exemplar blocks, and renders through small renderer traits so provider adapters can choose their final wire shape without owning prompt policy.

## Key API

- `PromptSpec::from_request(request, instruction_key, instruction_text)` captures the instruction key, task, schema, tools, persona, scratchpad, context membrane, prior context, and grounding claims.
- `PromptSpec::with_exemplars` and `ExemplarPlacement` control whether accepted exemplars live before or after the cache breakpoint.
- `Renderer`, `MarkerRenderer`, and `StructuredOutputRenderer` produce `RenderedPrompt` messages without binding to a provider SDK.
- `PromptAssembly` and `PromptBlock` expose the cache role of each prompt block for adapter-side caching decisions.

Path deps: `theorem-harness-core`, `serde`, and `serde_json`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-prompt
```

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
