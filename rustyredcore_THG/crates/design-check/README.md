# design-check

A static design-engineering checker and skill-pack payload: CSS and design-token lowering, WCAG contrast math, grid/type/motion checks, token linting, and component fixture artifact hashes, with honest "pending" declarations for the render-backed axes (`axe_render`, `apg_behavioral`). Lib plus `design-check` binary.

## Key API

- Rules/lowering: `design_rules() -> Vec<DesignRule>`, `lower_css(source_ref, css) -> Vec<DesignAtom>`, `lower_tokens_json(source_ref, json_text)`.
- Reports: `css_static_report(CssStaticInput) -> DesignCheckReport`, `token_lint_report(CssStaticInput)`, `fixture_reports()`.
- Color math: `parse_hex_color`, `relative_luminance`, `contrast_ratio`, `Color`.
- Hashing/payload: `pack_hash()`, `source_hash()`, `design_engineering_pack_payload(parent_hash: Option<&str>)`.
- `CssStaticInput` (css, optional token_json, `grid_base_px` default 4.0, `rem_px` default 16.0), `CheckerFinding`, `DesignCheckReport` (checker, pack_id, pack_hash, findings, passed/failed/pending/unsupported). `PACK_ID = "skill-pack:design-engineering-general-v0.1"`.

Bundled data: `src/skill/SKILL.md`; `src/fixtures/` (`static-fixture.css`, `tokens.json`, `apg-fixtures.json`, `corpus-packets.json`, `validation-tasks.json`). Deps: serde, sha2.

## CLI

Reads CSS/tokens from stdin. Modes: `--css-static` (default), `--token-lint`, `--lower-css`, `--lower-tokens`, `--fixture-report`, `--pack-payload` (with `--parent-hash HASH`), `--tokens-json JSON`, `--help`. Emits JSON.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p design-check
echo "a{color:#777;background:#fff}" | cargo run -p design-check -- --css-static
```

`tests/cli.rs` plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
