# Theorem Crate Publishing

This release lane makes the CommonPlace product repo independent from local
`Website/CommonPlace` + `Website/Theorem` path layouts without publishing
CommonPlace product code itself.

## Rule

Publish substrate and harness libraries. Keep product crates local:

- CommonPlace-owned: `crates/commonplace`, `apps/commonplace-api`,
  `crates/commonplace-desktop-runtime`.
- Published dependencies: RustyRed substrate, harness contracts/runtime, and
  receiver/server libraries needed by those product crates.

## Layers

Layer 0, pure contracts and tools:

- `theorem-harness-core`
- `prose-check`
- `design-check`

Layer 1, substrate core:

- `rustyred-thg-core`

Layer 2, smaller runtime support crates:

- `rustyred-thg-memory`
- `rustyred-thg-affordances`
- `rustyred-thg-connectors`
- `theorem-dispatch`
- `theorem-receiver`
- `theorem-browser-agent`
- `theorem-harness-runtime`

Layer 3, heavy server surface:

- `rustyred-thg-server`

Layer 3 is opt-in because it pulls the broad product server dependency forest.
Prefer publishing the smaller library layers first, then decide whether the
server should be a crate, a binary release, or a narrower CommonPlace adapter.

## Commands

Dry-run the default layers:

```bash
scripts/release-crates.sh --mode dry-run
```

`cargo publish --dry-run` and `cargo package` both resolve already-versioned
path dependencies through the registry. That means layer N can fully dry-run or
package only after layer N-1 has actually been published. After a prior layer is
published, package assembly can inspect the tarball shape without rebuilding:

```bash
scripts/release-crates.sh --mode package --layer 1 --no-verify
```

Run one layer:

```bash
scripts/release-crates.sh --mode dry-run --layer 0
```

Include the heavy server layer:

```bash
scripts/release-crates.sh --mode dry-run --include-heavy
```

Publish after the dry-runs are clean and crate ownership is confirmed:

```bash
CONFIRM_PUBLISH=yes scripts/release-crates.sh --mode publish --layer 0
```

`--mode publish` refuses `--allow-dirty`. For dry-run/package work,
`--allow-dirty` is available only to inspect an in-progress crate; do not use it
as release proof.
