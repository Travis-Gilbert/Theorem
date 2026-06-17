# A peer head can fully implement your claimed lane (source AND tests) after you announce a coordination_intent: re-read the files before you build, or you duplicate work that is already green

**Kind:** method
**Captured:** 2026-06-16
**Session signature:** `claude-code:travisgilbert (http-sse-transport + github-app handoffs)`
**Domain tags:** coordination, multi-head, crdt, co-build, coordination_intent, verify-before-build

## Trigger

I claimed Lane A (HTTP/SSE connector transport, `rustyred-thg-connectors`) and wrote a detailed `coordination_intent` footprint listing every file I would touch. I then spent several tool-cycles confirming the ureq 3.x API via context7 and planning a loopback stub HTTP server plus 6 new tests. When I re-read `transport.rs` to start editing, a peer head (`deepseek`/`claude` in `binding_active_head_set`) had ALREADY written the entire `HttpTransport` + isolated SSE parser + `connect_http` + `ConnectedTransport` + `connect_transport`, switched the `bridge.rs`/`invoke.rs` call sites, updated `lib.rs` re-exports, AND written the full test suite (29 tests covering all 8 acceptance criteria, green), including the exact stub-server + SSE-skip + session-id + invoke-over-http tests I was about to author. My planned build was already done on the shared tree. Separately, the `ureq` pin in my lane's `Cargo.toml` was changed under me from `2.12` to `3.3.0` (a different major API) between my edit and my next read.

## Rule

On this repo the heads share one git working tree plus a CRDT coordination substrate. After you write a `coordination_intent`, assume a peer head may implement your announced files before your next turn. ALWAYS re-read each target file immediately before editing it: if it already implements your intent, your job flips from build to VERIFY (run the suite, check diff scope, adversarially review) rather than re-author what is green. Treat your own lane's `Cargo.toml` and source as co-editable: if a dependency pin or a function changed under you, BUILD ON it (adopt the peer's `ureq 3.3.0` and write against that API) instead of clobbering back to your version. The coordination substrate converges; the source bytes do not, so re-read is the only safe pre-edit check.

## Evidence

- `read_intents_for_room` showed only my own intent (peers edit git-only and do not broadcast footprints), yet `transport.rs` came back as 395 lines of a complete implementation I never wrote.
- `cargo test -p rustyred-thg-connectors` returned 29 passed / 0 failed on the first run, including `http_bridge_completes_json_handshake_and_registers_tools`, `allowlist_can_fire_over_http_target_and_record_outcome`, and `register_with_http_target_persists_the_reach_for_planning` -- the three I had just flagged as "gaps to add."
- A `Cargo.toml` Edit failed with "File has been modified since read"; the re-read showed `ureq = "3.3.0"` where I had written `2.12`.
