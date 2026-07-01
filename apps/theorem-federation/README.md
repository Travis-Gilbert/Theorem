# theorem-federation

`theorem-federation` is the default-off direct peer federation lane for Theorem
nodes. It keeps the existing hosted sync lane intact and adds a second path for
node-to-node convergence.

Current scope:

- persistent Iroh `SecretKey` in the node data dir, with the public
  `EndpointId` as the peer identity
- sha256 identity to BLAKE3 transfer-link resolver for Prolly packs, cold
  objects, and documents
- length-bounded federation frames for `SubstratePeer` structured deltas, text
  `update_v1` bytes, and presence events
- deterministic iroh-gossip awareness topics by tenant and scope
- per-peer EMA trust policy keyed by EndpointId
- trust-gated receive helpers for inbound blob and delta acceptance
- `status`, `identity`, `doctor`, and `serve` commands

The crate is standalone (`[workspace]`) so the Iroh dependency family builds
independently of the main `rustyredcore_THG` workspace.

```bash
cargo test --manifest-path apps/theorem-federation/Cargo.toml
THEOREM_FEDERATION_ENABLED=1 theorem federation doctor --bind-endpoint
```

The integration tests bind loopback Iroh endpoints and prove authenticated QUIC
frame exchange, bidirectional SubstratePeer convergence, iroh-blobs provider/get
transfer, three-peer iroh-gossip awareness, and trust rejection before merge or
blob acceptance. Cross-network NAT, interrupted resume, cold graph checkout, and
forced relay-fallback acceptance still need a multi-node lab.
