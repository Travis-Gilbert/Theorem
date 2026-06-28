# rustyred-thg-server `/mcp` returns tool payloads wrapped in the MCP envelope (`result.structuredContent.{...}` + a duplicate JSON string in `result.content[0].text`), NOT flat `result.<field>` — a client parsing the flat shape silently gets nothing; prove the wire shape against the live node, because a mock you authored encodes your assumption, not the server's

**Kind:** gotcha
**Captured:** 2026-06-28
**Session signature:** `claude-code:travisgilbert (local-proxy Phase A: Valkey + SSD local node + substrate HttpMemorySource)`
**Domain tags:** theorem-proxy, mcp, rustyred-thg-server, wire-contract, testing, live-oracle, hippo_retrieve

## Trigger

I built `HttpMemorySource` in `apps/theorem-proxy` to retrieve ambient memory from the local node by POSTing a JSON-RPC `tools/call` for `hippo_retrieve` to `rustyred-thg-server`'s `/mcp`. I parsed the response as `result.candidates` (and a top-level `candidates` fallback), wrote a mock-`/mcp` axum test that returned exactly `{"result":{"candidates":[...]}}`, and got 12 green tests + clippy clean. It looked done. Then I stood up the real `rustyred-thg-server` (embedded, on the SSD) and curled the actual `hippo_retrieve`: `candidates: 0`. The payload was NOT flat — the real MCP `tools/call` envelope is `{"result":{"content":[{"type":"text","text":"<the JSON payload as a STRING>"}],"structuredContent":{<the JSON payload as an OBJECT: candidates, indexing, stats, trace, ...>}}}`. My parser read `result.candidates`, which does not exist, so `HttpMemorySource` fail-open'd to EMPTY against the real node: the proxy would have injected no live memory and nobody would have seen an error (fail-open hid it). The green mock had encoded my guess of the shape, not the server's actual shape.

## Rule

For any client that talks to a real MCP server over `/mcp` (`rustyred-thg-server`, `rustyred-embedded`'s `Engine::handle`, the harness), the tool result is wrapped: read `result.structuredContent.<field>` (an object) or parse the JSON string at `result.content[0].text` — never assume `result.<field>` is flat. Before trusting a parser, prove it against the LIVE server (an `#[ignore]` integration test that hits the running node and asserts non-empty), not only a self-authored mock: a mock returns whatever shape you typed, so a green mock + green unit tests can both be wrong in the same direction. When the client fails open (returns empty on any parse/transport miss), this class of bug is invisible without the live check — fail-open + wrong-shape = silent nothing. Build the mock's shape FROM a captured real response, not from the struct definition you expect.

## Evidence

- Real `hippo_retrieve` over `/mcp`: `result` keys = `['content','structuredContent']`; `structuredContent` keys = `['candidates','indexing','query','stats','tenant','trace']`. The encoded memory came back as `hippo:page:memory:sha256:... :: proxy-local-node-wiring` only after parsing `structuredContent`.
- Fix: `parse_candidates` now reads `result.structuredContent.candidates` first, then `result.candidates`, then top-level (commit `17f61fc8a`, `apps/theorem-proxy/src/memory.rs`).
- The mock test was realigned to the real envelope, and a live `#[ignore]` test (`cargo test --test substrate_memory -- --ignored`, hits `127.0.0.1:8380`) now guards it green with the node up. `coordination_context` and `encode` results are wrapped identically (`content[0].text` + `structuredContent`).

## Encoded in

- `docs/learnings/2026-06-28-rustyred-mcp-nests-tool-payload-under-structuredContent-verify-against-live-node.md` (this file)
- Memory: `[[harness-local-proxy]]` (progress note 2026-06-28).
