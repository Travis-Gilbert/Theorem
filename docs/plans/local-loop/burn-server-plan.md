# Burn serving path: scoping plan (CHK019, CHK021, CHK022)

The follow-on spike the Local Loop plan sequences last. The loop ships on
llama-server (CHK018); this brings up a Burn-native server behind the same
openai-compatible seam so the local model is fully Rust, with llama-server left
installed and cutover per-config and reversible. No GPU run is in scope for this
doc; it is the plan, not the execution.

## What already exists (do not rebuild)

- The reversible seam: `theorem-agentd` selects its model by config
  (`[model] provider`, `base_url`). A Burn server is a `base_url` swap; cutover is
  one config line and llama-server stays installed (CHK022's reversibility).
- Constrained decoding (CHK020): `theorem-agentd/src/constrained_decoding.rs`
  compiles the tool catalog into a token-level logit mask
  (`ToolGrammar::token_mask`). This is the catalog-specific half; it needs only a
  `Vocab` impl over the model's tokenizer to plug into the sampler.
- Burn + CubeCL are already in the repo's dependency graph (the reflexive
  `pairformer-burn-cubecl` feature in `rustyred-thg-adapters`), so the toolchain is
  proven on this machine (CubeCL launches on Metal/wgpu).

## The gap, as concrete tasks

### B1. Dependencies (CHK019)

- Add `burn-lm` (tracel-ai) and `CubeK`/`cubek` kernels to a new crate
  `apps/theorem-burn-server` (standalone Cargo root, like theorem-grpc), not the
  agentd crate. Pin revs deliberately; record them.
- Decide the backend: `burn-wgpu` (Metal on this Mac) vs `burn-candle`. wgpu
  matches the existing reflexive CubeCL path.

### B2. Weights (CHK021)

- On-disk weights are GGUF (`gemma-4-12B-it-qat-UD-Q4_K_XL.gguf`); llama-burn's
  reference loader expects safetensors. Convert once (HF `transformers` /
  `gguf-to-safetensors`), or implement a GGUF reader. Record the provenance + the
  exact Gemma 3 12B config (layers, heads, head_dim, rope_theta, vocab).
- Load into the llama-burn reference module shape (attention, RMSNorm, RoPE,
  SwiGLU). Verify tensor shapes against the config before any forward pass.

### B3. Sampler + constrained decoding wiring (CHK020 -> live)

- Implement `Vocab` over Gemma's tokenizer (id -> piece bytes).
- At each decode step: run `ToolGrammar::token_mask(decoded, &vocab)`, set the
  logits of disallowed tokens to `f32::NEG_INFINITY`, then sample. This is the
  "token-level logit mask at the sampler" CHK020 names; the compiler is built and
  tested, this wires it in.
- Confirm first whether burn-lm ships any structured-output hook; if so, prefer
  it and keep the mask as the fallback. (CHK020's "confirm whether burn-lm ships
  structured output" step.)

### B4. The openai-compatible server (CHK019)

- Axum server exposing `POST /v1/chat/completions` with the subset
  `theorem-agentd` sends (messages, temperature, max_tokens, tools). Translate to
  the Burn forward+sample loop. Return the same `choices[0].message` /
  `tool_calls` shape `model.rs::parse_chat_completion` already parses, so the
  daemon needs no change.
- Bind `127.0.0.1:8081` (avoid llama-server's 8080) so both can run side by side
  during parity.

### B5. Parity gate (CHK022)

- Replay the daemon's own ledger transcripts (`.theorem/agentd-token-ledger.jsonl`,
  the label factory) through both servers; compare the decided tool call / final
  text per turn. Parity = same decision on the transcript set (not byte-identical
  prose).
- Cut over by config only when parity holds; leave llama-server installed.

## Why it is not done here

`burn-lm` + `CubeK` are not yet dependencies in this repo, the weights are GGUF
not safetensors, and parity needs a GPU forward pass. Those are the B1/B2/B5
inputs; this environment has none of them wired. The loop generating traces (the
ledger) is the precondition the original plan named, and that is now in place.
