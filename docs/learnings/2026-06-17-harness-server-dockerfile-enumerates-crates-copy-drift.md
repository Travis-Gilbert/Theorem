# A new workspace path-dep silently bricks the Railway `theorem` service because `apps/theorem-harness-server/Dockerfile` COPYs crates one-by-one and nobody updates it — align it to theorem-grpc's whole-`crates`-dir COPY

**Kind:** postmortem
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (hipporag+search-rerank verify / railway restore)`
**Domain tags:** railway, dockerfile, cargo, path-dependency, copy-drift, theorem-harness-server, theorem-dispatch, deploy

## Trigger

The `theorem` Railway service (which builds `theorem-harness-server`) had been FAILED since ~04:06 UTC, exit code 101, on every deploy from commit `bc27704f` onward. The build log root cause:

```
error: failed to load manifest for dependency `theorem-dispatch`
  failed to read `/app/rustyredcore_THG/crates/theorem-dispatch/Cargo.toml`
  No such file or directory (os error 2)
```

`apps/theorem-harness-server/Cargo.toml` had gained `theorem-dispatch = { path = "../../rustyredcore_THG/crates/theorem-dispatch" }` (Dispatch v2 job board), but `apps/theorem-harness-server/Dockerfile` COPYs each crate **individually** (`COPY rustyredcore_THG/crates/theorem-harness-core …`, one line per crate) and was never given a line for `theorem-dispatch`. The build context lacked the crate, so cargo couldn't read its manifest. `RustyRedCore - Theorem` and `theorem-grpc` deployed fine from the same git push because their Dockerfiles already copy the whole `crates` dir.

## Rule

A per-crate-enumerating Dockerfile is a latent landmine: it breaks the next time anyone adds a workspace path-dep to that app, with no local signal (the workspace builds fine; only the Railway build context is short a crate). Two responses:

1. **Immediate:** add the matching `COPY rustyredcore_THG/crates/<new-crate> ./rustyredcore_THG/crates/<new-crate>` line (and recurse into the new crate's own path-deps).
2. **Durable (preferred):** switch the enumerating Dockerfile to copy the whole dir, exactly as `theorem-grpc` did in commit `622d202e` (`fix(theorem-grpc): copy whole crates dir in Dockerfile, fixes exit-101 build drift`):
   ```dockerfile
   COPY rustyredcore_THG/crates ./rustyredcore_THG/crates
   ```
   so the next path-dep addition cannot drift.

When you add a workspace path-dep to ANY `apps/*` crate, grep every service Dockerfile for its COPY style before assuming the deploy is safe.

## Evidence

- Failed deploys: `79c4bac1` (commit `42cb4539`, 04:11), `567224a7` (commit `bc27704f`, 04:06).
- Fix: commit `51228e2e` added the one missing `COPY …/theorem-dispatch …` line; deploy `4fe90174` → SUCCESS; `https://theorem-production.up.railway.app/` → HTTP 404 (server up).
- Precedent for the durable fix: `theorem-grpc` commit `622d202e` switched to whole-dir COPY for the identical exit-101 failure mode. `theorem-harness-server` is the same Dockerfile shape still enumerating, so it remains one path-dep away from the next break until converted.
