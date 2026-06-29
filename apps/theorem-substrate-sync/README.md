# theorem-substrate-sync

`theorem-substrate-sync` is the local-to-hosted substrate sync daemon for one
Theorem tenant. It is a standalone Cargo root, not a `rustyredcore_THG`
workspace member.

It owns three roles from `docs/plans/substrate-sync/`:

- Prolly version-pack rounds through the existing `rustyred_thg_graph_version_*`
  MCP verbs, using `graphql_mutate` bulk writes to apply checked-out snapshots.
- A Valkey-backed outbox and stream tail over the existing `stream_publish`,
  `stream_subscribe`, and `stream_read` verbs.
- Status and manual trigger endpoints for the local launcher and doctor flow.

The daemon is default-off. Launch it through
`THEOREM_SYNC_ENABLED=1 apps/theorem-proxy/scripts/start-proxied-session.sh` or
run it directly with:

```bash
cargo run --manifest-path apps/theorem-substrate-sync/Cargo.toml -- serve
```
