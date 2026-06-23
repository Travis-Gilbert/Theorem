# Servo Browser-Use, build step one: parity on Servo (job-007)

**Repo:** Travis-Gilbert/theorem
**Audience:** Claude Code + Codex, building as one agent
**Plan home:** docs/plans/servo-browser-use-agent/
**Builds on:** the parent HANDOFF.md (bd0b7e1d), which is largely built. This is the execution slice that brings the ported stack to feature parity with Browser Use, on the Servo engine.
**Engine assumption:** the Servo accessibility PRs (interactive roles, node properties, accessible-name, bounds, action support) are treated as DONE. They land in the fork as commits plus harness notes; this slice consumes them through the reader. Where a property is not yet upstreamed, the reader degrades per D5 rather than blocking.
**Job linkage:** job-007, kind Feature, priority P0, target_head Either.

## North star

Everything Browser Use does, the ported perceive/govern/afford stack already does at the contract level. This slice closes the concrete action and capability gaps so a task that runs on Browser Use runs here, driven by the in-process Servo engine reading the accessibility tree rather than CDP from outside. Parity is the floor, not the ceiling; steps two and three build past it.

## Browser Use parity surface (the bar, grounded)

Browser Use ships roughly 45 actions. The load-bearing set: navigate, click, type/input, scroll, send_keys, dropdown_options, select_dropdown, upload_file, extract (LLM over page content), screenshot, switch_tab, close_tab, go_back, wait, done; plus structured-output validation against a schema, sensitive-data masking in logs and model context, domain restriction, and download detection on click. The parent plan's Action enum (Click, Type, Select, Scroll, Back, Forward, WaitFor, Submit) and the action_rail catalog cover the core. This slice adds what is missing.

## Deliverables

### D1: the PageState reader (agent-layer, the spine)
A reader in rustyred-web that consumes the Servo accessibility TreeUpdate stream and produces PageState exactly per the parent contract: url, title, distilled text, and interactive_elements [{element_id, role, name, value, bbox, visible}].
- element_id is the stable Servo NodeId (id_for_opaque assigns one persistent id per DOM node; do not invent a second id space).
- Consume the TreeUpdate diff as the change signal: changed nodes arrive incrementally, so PageState updates without a full re-walk. This is the engine-native equivalent of a mutation observer, and it is free.
- visible and true clickability: resolve from bounds plus paint order layer-side; if occlusion needs a hit test the engine does not expose, note it as a follow-up and ship with bounds-based visibility.
- Verify against the engine: confirm which AccessKit node fields the fork now populates (role, value, bounds, checked/selected) before wiring; the reader reads what exists and leaves absent fields None.

### D2: the Action executor (engine-native act)
Implement the parent Action enum against Servo's action path (AccessKit ActionRequest -> DOM activation), not synthetic events:
- Click(element_id), Type(element_id, text), Select(element_id, value), Scroll(delta), Back, Forward, Submit, WaitFor(condition).
- Add the parity gaps as Action variants: SendKeys(sequence) for raw key navigation (Browser Use's documented fallback when a click fails), SelectOption for dropdowns by visible text or value, and ScrollToElement(element_id).
- Each action resolves the element by stable id, performs through the engine action path, and returns an observe() delta so the loop sees the result.
- Verify whether a perform_action path exists in the fork yet (the layout file references ActionRequest but implements no handler); if the engine side is still thin, this deliverable includes the minimal DOM-activation routing, coordinated as a fork commit.

### D3: extract, upload, tabs, capture (parity actions)
- extract(schema): the existing web_consume read primitive feeding a model pass that returns structured JSON validated against a caller schema. This is Browser Use's extract plus its structured-output validation in one.
- upload_file(element_id, path): set the file input from the receiver/runtime's available-paths list; gate behind PermissionPolicy.allow_write and a path allowlist (no arbitrary filesystem reach).
- Tabs: open_tab(url), switch_tab(id), close_tab(id), list_tabs over the Servo webview set; PageState tracks the active tab.
- screenshot/render() -> image for the vision fallback and receipts.

### D4: sensitive-data and domain safety (parity, native via Context Command)
- Domain restriction: a permitted-domains set on the ContextCommandState; navigation or action outside it is refused. This is RetrievalPolicy/PermissionPolicy territory, already in the stack.
- Sensitive-data masking: a sensitive_data map (domain-scoped) whose values are substituted into Type/upload without the literal entering trace events, model context, or receipts; the trace records a masked marker. Mirrors Browser Use's has_sensitive_data plus domain-scoped secrets.
- Download detection: a Click that triggers a download returns download metadata in the observe() delta.

### D5: graceful degradation against engine maturity
Where the fork has not yet populated an interactive role or property, the reader emits the node with role GenericContainer and the action executor falls back to SendKeys-style keyboard navigation (Browser Use's own documented workaround). A degraded element is marked degraded:true in PageState so the driving model knows to prefer keyboard or vision. Engine maturity raises fidelity; it never blocks the loop.

## Acceptance criteria

1. PageState for a real form page lists its buttons, links, and inputs with stable ids, names, values, and bounds, sourced from the Servo accessibility tree.
2. A TreeUpdate after a DOM mutation updates PageState incrementally without a full re-walk.
3. The executor clicks a button and types into a field by stable id through the engine action path, and observe() reflects the change.
4. extract(schema) returns JSON validated against the schema on a content page.
5. upload_file sets a file input from an allowlisted path and refuses a non-allowlisted one.
6. Tab open/switch/close work over the Servo webview set; PageState tracks the active tab.
7. A sensitive_data value is typed into a field and appears nowhere in the trace, model context, or receipt; the trace shows a masked marker.
8. Navigation to a domain outside the permitted set is refused.
9. A browse_for_me task that Browser Use can complete (search, open result, extract 5 items, write file) completes here end to end and emits a BrowsingRun.
10. A page element the engine has not yet rolled is marked degraded and is still operable by keyboard fallback.

## Fences

- No CDP, no external browser, no Playwright. The engine is embedded Servo.
- No second element-id space; the Servo NodeId is the id.
- This slice is parity plus the engine-native execution path. Servo-specific new abilities are step two (job-008); Theorem/Theseus-only abilities are step three (job-009).
- Standing no-graph-view fence holds (this is the agent, not a graph UI).

## Where it rides

rustyred-web owns the engine, the reader, and the executor (all outbound web I/O). The perceive/govern/afford stack in core calls them per the parent plan. MCP surface (browse_with_me, browse_for_me, web_consume) already specified in the parent; this slice fills their executor.
