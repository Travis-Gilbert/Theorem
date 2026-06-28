# design-check

A static design-engineering checker and skill-pack payload: CSS and design-token lowering, WCAG contrast math, grid/type/motion checks, token linting, and component fixture artifact hashes, with honest "pending" declarations for the render-backed axes (`axe_render`, `apg_behavioral`). It also carries the browserless Design Scout callable validator cut, pinned to `dembrandt/dembrandt` commit `e7a05893d5d045c01a07008cd616035ad29e7154`: normalized design facts, deterministic audit findings, drift reports, DTCG/Tailwind token output, and a self-contained HTML report. Lib plus `design-check` binary.

## Key API

- Rules/lowering: `design_rules() -> Vec<DesignRule>`, `lower_css(source_ref, css) -> Vec<DesignAtom>`, `lower_tokens_json(source_ref, json_text)`.
- Reports: `css_static_report(CssStaticInput) -> DesignCheckReport`, `token_lint_report(CssStaticInput)`, `fixture_reports()`.
- Design Scout facts/tools: `design_fact_set_from_json`, `design_audit`, `design_drift`, `design_tokens_dtcg`, `design_tokens_tailwind`, `design_html_report`, `design_scout_parity_receipt`.
- Color math: `parse_hex_color`, `relative_luminance`, `contrast_ratio`, `delta_e2000`, `apca_contrast_lc`, `Color`.
- Hashing/payload: `pack_hash()`, `source_hash()`, `design_engineering_pack_payload(parent_hash: Option<&str>)`.
- `CssStaticInput` (css, optional token_json, `grid_base_px` default 4.0, `rem_px` default 16.0), `CheckerFinding`, `DesignCheckReport` (checker, pack_id, pack_hash, findings, passed/failed/pending/unsupported). `PACK_ID = "skill-pack:design-engineering-general-v0.1"`.

Bundled data: `src/skill/SKILL.md`; `src/fixtures/` (`static-fixture.css`, `tokens.json`, `apg-fixtures.json`, `corpus-packets.json`, `validation-tasks.json`). Deps: serde, sha2.

## CLI

Reads CSS/tokens/facts from stdin. Modes: `--css-static` (default), `--token-lint`, `--lower-css`, `--lower-tokens`, `--audit-facts`, `--drift-facts --baseline-json JSON`, `--tokens-dtcg`, `--tokens-tailwind`, `--html-report`, `--fixture-report`, `--pack-payload` (with `--parent-hash HASH`), `--tokens-json JSON`, `--help`. Emits JSON; `--html-report` emits the HTML string as JSON.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p design-check
echo "a{color:#777;background:#fff}" | cargo run -p design-check -- --css-static
cat crates/design-check/src/fixtures/dembrandt-extraction-synthetic.json | cargo run -p design-check -- --audit-facts
```

`tests/cli.rs` plus inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
