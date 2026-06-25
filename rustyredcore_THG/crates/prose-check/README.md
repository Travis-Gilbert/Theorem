# prose-check

A deterministic writing-engineering style checker that emits a content-addressed `StyleReceipt` and a skill-pack payload for Theorem harness receipts. Lib plus `prose-check` binary.

## Key API

- `check(text, register, source_identifiers) -> StyleReceipt`, `check_with_baseline(text, baseline, register, source_identifiers)`.
- `pack_hash() -> String` (sha256 over all rule files, memoized), `writing_engineering_pack_payload(parent_hash: Option<&str>) -> Value`.
- `Register` (`Plain`/`Spare`/`Wire`); `StyleReceipt` (register, tokens, reduction, fidelity, passive_rate, adverb_rate, clutter_hits, nominalization_rate, sentence_mean/stdev, flesch_kincaid, em_dash_count, clarity_breaks, code_spans, pack_hash); `Span`, `Fidelity`, `ClutterHit`.
- `PACK_ID = "skill-pack:writing-engineering-prose-v0.1"`, `TOKENIZER_NAME = "cl100k_base_estimate"`.

Rule data (`src/rules/`, all `include_str!`): `directive.txt`, `registers.txt`, `clutter.tsv`, `redundant-pairs.tsv`, `latinate-swaps.tsv`, `adverb-whitelist.txt`, `hedges.txt`, `wire-abbrev.tsv`. The pack hash is computed over these eight files. Deps: serde, sha2.

## CLI

Reads text from stdin. Flags: `--register plain|spare|wire`, `--identifiers a,b`, `--pack-payload` (with `--parent-hash HASH`), `--status <x>`, `--help`. Emits a JSON receipt or pack payload.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p prose-check
echo "some prose" | cargo run -p prose-check -- --register plain
```

`tests/cli.rs` plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
