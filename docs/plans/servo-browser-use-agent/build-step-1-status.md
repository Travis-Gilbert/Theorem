# Build step one (job-007) status: the perception + governance half

Author: claude-code. Sibling to `build-step-1-parity.md` (the spec) and
`CONTRACT-RECONCILIATION.md`. Records what the reader+governance half delivers,
honestly mapped to the acceptance criteria, after an adversarial 4-agent review.

## The seam (per Travis's "reader vs executor" split)

- **claude-code took**: D1 (the PageState reader), D3/D4/D5 agent-layer parity
  (extract-schema validation, upload allowlist, tabs model, domain restriction,
  sensitive-data masking, download detection, degradation marking + keyboard
  fallback plan). One new module `rustyred-web/src/browser_perception.rs`; one
  additive field `InteractiveElement.degraded` (serde-default false).
- **Open for codex (the executor half)**: `browser_engine.rs::act()` driven by
  AccessKit `ActionRequest` (not synthetic events); the parity `BrowserAction`
  variants `SendKeys` / `SelectOption` / `ScrollToElement`; `upload_file` setting
  the file input (gated by `resolve_upload_path` + `PermissionPolicy.allow_write`);
  tab webview control bound to `TabSet`; and the Servo-fork `perform_action`
  handler the grounding flagged (`accessibility_tree.rs` references `ActionRequest`
  but implements none). The executor act() and the BrowserAction enum were left
  byte-identical to HEAD.

## The engine boundary (the central GAP, by design)

The reader consumes `A11yTreeUpdate`, a faithful serde projection of
`accesskit::{TreeUpdate, Node, NodeId, Rect}`. The real
`accesskit::TreeUpdate -> A11yTreeUpdate` conversion is implemented and tested
behind the optional `accesskit` feature (`A11yTreeUpdate::from_accesskit`,
verified against accesskit 0.24). **It has no live caller**: the Servo embedder
hook that would feed a real TreeUpdate into `AccessibilityReader` does not exist
yet (the spec treats the Servo a11y PRs as a deferred assumption). So everything
is exercised against synthetic/real-accesskit test input, never a live engine.
This is the expected GAP for this slice; wiring it is the embedder's job (CI-only
libservo), and the accesskit version pin must be reconciled with Servo's then.

## Acceptance map

| # | Criterion | Verdict | Note |
|---|-----------|---------|------|
| 1 | PageState lists controls w/ stable ids, names, values, bounds, sourced from the Servo a11y tree | PARTIAL | Reader/projection COVERED + tested (stable NodeId id, bounds, name/value); "sourced from LIVE Servo" is GAP (embedder hook absent). |
| 2 | TreeUpdate updates PageState incrementally, no full re-walk | COVERED | Stateful reader, incremental upsert + reachability prune + before/after diff; tested (change one node; remove via parent children). |
| 3 | Executor clicks/types by stable id through the engine action path | GAP | Executor half (codex). This slice leaves a clean extension point (NodeId id, degraded flag, keyboard-fallback plan). |
| 4 | extract(schema) returns JSON validated against the schema | PARTIAL | Schema validator COVERED + hardened (see holes); the "LLM over page content" pass and live-page sourcing are not in this slice. |
| 5 | upload_file sets a file input from an allowlisted path, refuses otherwise | PARTIAL | The allowlist gate (`resolve_upload_path`) is COVERED + adversarially tested; "sets the file input" + allow_write gate are the executor half (GAP). |
| 6 | Tab open/switch/close; PageState tracks the active tab | PARTIAL | `TabSet` model COVERED + tested; the webview binding and a PageState active-tab field are the executor/session seam (GAP). |
| 7 | A sensitive value appears nowhere in trace/context/receipt; trace shows a masked marker | PARTIAL | Masking primitive COVERED + hardened (leak-free, see holes); the actual trace/receipt wiring is the executor/ContextCommandState seam (GAP). |
| 8 | Navigation outside the permitted set is refused | PARTIAL | `DomainPolicy` classifier COVERED + hardened; enforcement in `navigate()` is the executor seam (GAP). |
| 9 | A browse_for_me task completes end to end and emits a BrowsingRun | GAP | Needs executor + live embedder. The "validate 5 extracted items" sub-part is COVERED via the schema validator. |
| 10 | An unrolled element is marked degraded and stays keyboard-operable | COVERED | `degraded` marking + `keyboard_fallback_for` plan COVERED + tested; the keyboard *execution* is the executor. |
| Fence | No CDP / external browser / Playwright | COVERED | None introduced; the engine path is the AccessKit DTO. |
| Fence | No second element-id space; NodeId is the id | COVERED | `element_id = NodeId.to_string()`; verified on both DTO and real-accesskit paths. |
| Fence | Additive contract change only | COVERED | `git diff HEAD` on browser_engine.rs = exactly the additive `degraded` field + its 5 reader initializers; `page_state_payload` / `page_observation_from_state` unaffected; server 45+157 and mcp green. |

## Holes the review found, and the fixes (all in this half, all now tested)

1. **Sensitive masking leaks (HIGH, criterion 7)** - the naive replace-loop leaked
   on (a) substring/prefix secrets, (b) a wildcard `*` secret shadowed by a
   same-key domain secret, (c) a secret value colliding with the marker template.
   Fixed by rewriting `mask` as a single longest-match-first forward scan
   (tokenizer) over the union of `*`+domain secrets, emitting markers to a buffer
   that is never re-scanned. Tests: `masking_does_not_leak_when_one_secret_is_a_substring_of_another`,
   `masking_redacts_a_wildcard_secret_even_when_a_domain_shares_the_key`,
   `masking_is_not_corrupted_by_a_secret_that_collides_with_the_marker`.
2. **Schema validator bypasses (HIGH, criterion 4)** - unknown `type` accepted
   anything and skipped recursion; `required`/`properties` without `type` on a
   non-object silently passed; tuple-form `items` ignored. Fixed: unknown type is
   an error and still recurses; non-object with required/properties errors;
   tuple-form items validated positionally. Tests: `schema_unknown_type_*`,
   `schema_required_without_type_*`, `schema_tuple_form_items_*`.
3. **Domain policy (MED, criterion 8)** - trailing-dot FQDN asymmetry let the
   absolute form read as off-domain; a blank allowlist entry bricked all
   navigation. Fixed: trailing-dot normalised both sides; blank entries ignored
   (all-blank reads as unrestricted). Tests: `domain_trailing_dot_*`,
   `domain_blank_allowlist_*`.
4. **Disabled controls surfaced as actionable (LOW)** - now excluded from
   interactive_elements. Test: `disabled_controls_are_not_surfaced_as_interactive`.
5. **Root-without-node wiped the tree (LOW)** - guarded; a declared-but-unsupplied
   root keeps the partial tree. Test: `root_declared_without_its_node_does_not_wipe_the_tree`.

## Reconciliation items for codex (executor half)

- `browser_engine.rs::extract` takes a schema and stores it unread; route it
  through `extract_structured` / `validate_against_schema` so the two extract
  surfaces share one contract.
- Link value semantics: the HTML reader puts the href in `InteractiveElement.value`;
  the accesskit reader puts the accesskit `value` (not an href). The executor's
  link-Click branch (`navigate(value)`) must not assume value is a URL for
  accesskit-sourced elements (click by NodeId via ActionRequest instead).
- "PageState tracks the active tab" wants an additive `PageState` field stamped
  from `TabSet::active()` when the executor wires tabs (and emitted in
  `page_state_payload`).

## Validation receipts

- `cargo test -p rustyred-web` (default): 85 + 2 + 12 green, 0 warnings.
- `cargo test -p rustyred-web --features accesskit`: 24 browser_perception green
  (incl. the real-accesskit-0.24 conversion), 0 warnings.
- `cargo test -p rustyred-thg-server -p rustyred-thg-mcp`: server 45+157, mcp green
  (the additive `degraded` field does not break the consumers).
- `cargo build --workspace` fails only on the root `rustyredcore_thg` PyO3 cdylib
  link step (pre-existing; per-crate `-p` builds are the supported path).

## Embedder hook: live sourcing (criterion 1 GAP -> addressed, CI-pending)

Written in `apps/browser` (the Servo embedder, a standalone CI-only crate). This
is the piece that turns "from_accesskit has no live caller" into a real one and
closes the "sourced from the Servo accessibility tree" half of criterion 1.

Grounded in the actual Servo source at the pinned rev
`b891f04d0819272b27e80ac975e2e57d3cb9e66b` (every API verified, not guessed):
- The embedder API **already exposes** the a11y tree:
  `WebViewDelegate::notify_accessibility_tree_update(WebView, accesskit::TreeUpdate)`.
  No Servo fork needed for the plumbing.
- Enable: `Preferences::accessibility_enabled = true` on the `ServoBuilder` plus
  `WebView::set_accessibility_active(true)` (returns `None` unless the pref is set).
- `WebView::page_title() -> Option<String>`, `WebView::url() -> Option<Url>`.
- Version: Servo pins `accesskit = { version = "0.24.0", features = ["serde"] }`,
  matching `rustyred-web`'s `accesskit` feature pin (0.24) and `apps/browser`'s new
  `accesskit = "0.24"`; the `TreeUpdate` type unifies with `from_accesskit`.

What landed:
- `apps/browser/Cargo.toml`: `accesskit = "0.24"` + a direct `rustyred-web` dep with
  `features = ["accesskit"]`.
- `apps/browser/src/main.rs`: `A11ySmokeDelegate` implementing
  `notify_accessibility_tree_update` (converts via `A11yTreeUpdate::from_accesskit`
  and feeds `AccessibilityReader`), plus a `--headless-a11y-smoke` mode that enables
  a11y, loads the smoke page, waits for live page content, and reports the
  a11y-sourced `PageState`.
- `.github/workflows/servo-browser.yml`: a `--headless-a11y-smoke` CI step.

Honest scope of the smoke:
- The robust assertion is `saw_page_content`: the raw `accesskit::TreeUpdate` Servo
  delivered carried the page's own text. This proves end-to-end live sourcing
  (engine -> delegate -> from_accesskit -> reader) without depending on reader
  assembly. `from_accesskit` + the reader also run on the real tree (no panic).
- NOT asserted (printed for evidence): the flat reader's `distilled_text` /
  `interactive_elements`. Two named follow-ups:
  1. **Multi-tree grafting.** Servo sends a grafted multi-tree update (a WebView
     `ScrollView` tree + the grafted document subtree, each with an independent
     NodeId space; `webview.rs::notify_document_accessibility_tree_id`). The flat
     `AccessibilityReader` ignores `tree_id` and does not assemble the graft, so
     reachability-from-root is unreliable across trees. The reader should become
     `tree_id`/graft-aware (or the embedder should select the document subtree).
  2. **Interactive roles.** Servo does not roll Link/Button/Input at this rev
     (`tests/accessibility.rs` exercises structural roles only), so
     `interactive_elements` stays thin until the fork rolls interactive roles.

Verification: `apps/browser` is CI-only (libservo ~30 min); cannot be compiled
locally. Verify by triggering `servo-browser.yml`, which requires the changes to
be pushed. As of writing, a clean isolated push is blocked: `browser_perception.rs`
requires the `degraded` field in `browser_engine.rs`, which now also carries
Codex's live executor WIP, so the combined job-007 (reader+governance+executor+
embedder) should be committed together at a coordinated boundary, then CI run.
