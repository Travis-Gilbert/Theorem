# rustyred-proxy

`rustyred-proxy` is the proxy-only local node for Anthropic Messages clients.
It serves the same implementation as `theorem-agentd --proxy` without starting
the local model loop.

```bash
cargo run -p rustyred-proxy -- \
  --proxy-port 8484 \
  --proxy-data-dir "/Volumes/SSD Samsung/theorem-local-proxy"
```

It exposes:

- `POST /v1/messages` for Anthropic Messages passthrough.
- hidden resident harness affordances on `/v1/messages`: direct
  `compute_offload.route_operation` plus `tool_search`/`describe`/`invoke`,
  with tier-two and tier-three holds represented as approval-required tool
  results.
- corpus-gated local/upstream cascade routing when
  `THEOREM_PROXY_LOCAL_ANTHROPIC_UPSTREAM` and
  `THEOREM_PROXY_CASCADE_CALIBRATION` are set.
- advisory verification injection from
  `<proxy-data-dir>/verification_claims.json` or
  `THEOREM_PROXY_VERIFICATION_CLAIMS`.
- `GET /v1/tool-result-fetch` for byte-slice recovery of sampled tool output.
- `POST/GET /v1/presence` for native local co-presence.
- `POST /v1/presence/footprint` and `DELETE /v1/presence/footprint` for
  pending-edit footprints.
- `POST /v1/presence/would-overlap` for peer overlap checks before edits.
- `/v1/agents/*` compatibility endpoints for older harness-room integrations.

The wrapper command `theorem proxy` prefers this binary when installed and
falls back to `theorem-agentd --proxy` or Cargo from a source checkout.
