# theorem-harness-runtime

Rust-native Theorem harness runtime: GraphStore-backed event log and run persistence.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
```

## Live provider heads

For the API-backed composed-agent binding, set:

```bash
THEOREM_AGENT_HEADS=deepseek,mistral,minimax
DEEPSEEK_API_KEY=...
MISTRAL_API_KEY=...
MINIMAX_API_KEY=...
```

Current default models are `deepseek-v4-flash`, `mistral-large-latest`, and
`MiniMax-M3`. Override with `DEEPSEEK_MODEL`, `MISTRAL_MODEL`, or
`MINIMAX_MODEL` when an operator wants a specific provider model. Provider
endpoint overrides stay credential-free and use `DEEPSEEK_CHAT_URL`,
`MISTRAL_CHAT_URL`, or `MINIMAX_CHAT_URL`.

`THEOREM_HEAD_INVOKER=real` selects live provider calls in the MCP composed-agent
handler; tests default to `fake` unless they opt in. `HeadTransport::Local`
uses `THEOREM_LOCAL_OPENAI_URL` (default `http://127.0.0.1:8080/v1/chat/completions`)
and allows no bearer token for llama-server/Gemma. `HeadTransport::Hosted` uses
`THEOREM_HOSTED_OPENAI_URL`, `THEOREM_LITELLM_CHAT_URL`, or
`THEOREM_LITELLM_BASE_URL` and requires the configured credential reference.

Part of the `rustyredcore_THG` Cargo workspace. See the crate table in [CLAUDE.md](../../../CLAUDE.md) for how this fits the substrate. This README is generated from the crate's `Cargo.toml` description and `//!` module docs; edit those and regenerate with `scripts/gen-crate-readmes.sh`.
