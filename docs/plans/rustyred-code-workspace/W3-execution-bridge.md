# W3: Execution bridge (materialize-to-run)

The toolchain runs the code. The embedded engine materializes the relevant subtree
into a real OS directory, runs the real toolchain (`cargo`, `python`, `node`) as a
real process in OS-level isolation, and syncs the toolchain's edits back into the
`DocTree`. This is the layer-6 owner and the "run" half of the DocTree-primary,
materialize-to-run decision.

Dependency edges: **W0 precedes W3** (materialize-to-run needs a tree to
materialize and the workspace seam to read it). W3 is independent of W1 and W2,
though it converges with W2 at "run, then commit, then push."

## Thesis

An application-level filesystem cannot execute code: compilers, interpreters, and
runtimes read the real OS filesystem and spawn real processes through syscalls. The
fix is the standard cloud-dev pattern (durable workspace store plus ephemeral
execution sandbox plus sync/mount), not becoming an OS. Execution as real processes
already exists in the system (the receiver spawns `claude -p` / `codex exec`); W3 is
not a new capability to invent, only a new thing to point at the working tree, plus
the materialize and sync-back steps that do not exist yet.

## What already exists (reuse, do not rebuild)

- **Real-process execution with capture and timeout.** `run_proof(plan: &ProofPlan)
  -> ProofReceipt` at
  `rustyredcore_THG/crates/theorem-receiver/src/local_exec.rs:87`: spawns a command
  with file-based stdout/stderr capture (avoids pipe deadlock), polls with a deadline,
  kills on timeout, returns a `ProofReceipt` (`:61`) tagged with a trust tier.
  `ProofPlan` (`:35`) is `{ command, args, cwd, timeout }`. This is the execution
  primitive; W3 generalizes it from "proof/test command" to "any toolchain command."
- **An isolation backend already exists.** `SandboxRuntime` trait at
  `rustyredcore_THG/crates/theorem-receiver/src/sandbox_exec.rs:59`:
  `provision(request) -> SandboxHandle`, `run(handle, plan) -> ProofReceipt`,
  `put_files` / `get_files`, `destroy`. `OpenSandboxRuntime` (`:72`) is the first
  backend (an HTTP sidecar). The receipt shape is backend-independent
  (`proof_receipt_from_execd_body`), so local and sandbox runs return the same
  `ProofReceipt`.
- **Spawn plans strip secrets.** `SpawnPlan` (`spawn.rs:29`) + `command_from_plan`
  (`:49`) already `env_remove` stripped vars (`ANTHROPIC_API_KEY` for head spawn).
  The pattern W3 extends for toolchain env policy.
- **Worktree mapping as a fence.** `worktree_for` (`config.rs:362`) maps a repo to a
  local path and is a security fence (a job for an unmapped repo is never claimed).

## The gaps W3 closes

- **No materialization layer.** `SandboxRuntime::put_files` exists but is not wired to
  materialize a codebase from the `DocTree` before exec. W3 adds the
  materialize-from-DocTree and sync-back-into-DocTree steps.
- **`ProofPlan` is verification-specific.** It runs test/proof commands, not arbitrary
  toolchain commands, and captures one command's output, not build artifacts. W3 adds
  a generic `RunPlan` (or generalizes `ProofPlan`) plus artifact handling.
- **No source/artifact boundary at run time.** A build writes thousands of transient
  files (`target/`, `node_modules/`); re-indexing them is fatal to the loop. W3
  enforces the boundary: only source syncs back into the `DocTree`; build output stays
  on throwaway disk.

## What to build

In a new module of `apps/rustyred-workspace` (the W0 crate), or a sibling crate
path-depping into `theorem-receiver` and `rustyred-embedded`:

### W3.1: materialize-from-DocTree

- `materialize(engine: &Engine, scope: &PathSpec, dir: &Path) -> MaterializeReceipt`:
  walks the `DocTree` subtree (via the W0 `list_paths` seam + `fs_read`) and writes
  real files into `dir`. For the URL-import case this is a no-op over the already
  cloned tree; for the DocTree-primary case it is the projection that gives the
  toolchain real files.

### W3.2: run the toolchain

- Generalize the execution primitive to a `RunPlan { command, args, cwd, timeout,
  env_policy }` and reuse `run_proof`'s capture/timeout/kill mechanics (or call it
  directly for the local backend). The driving command is caller-supplied (`cargo
  build`, `pytest`, `npm test`), mirroring how the receiver already takes the head
  command as data.
- **Isolation choice (resolved):** reuse the `SandboxRuntime` trait, do not invent a
  parallel one. The first backend is the existing `OpenSandboxRuntime` HTTP sidecar
  (keeps the engine Servo-free and toolchain-free); a `LocalProcessSandbox` backend
  (a thin `run_proof` over a temp dir with env stripping) is the dev/no-sidecar path.
  In-process and RunPod variants swap behind the same trait without touching W3's
  bridge. This is the north star's open "where does isolation come from" answered:
  the trait already abstracts it; W3 ships two backends behind it.

### W3.3: sync-back into the DocTree

- After a run that edits source (the agent's tool ran `cargo fix`, or a formatter, or
  the agent itself wrote files), diff the materialized dir against the `DocTree` and
  write changed **source** files back via the W0 batch path, which fires the W1
  on-write maintenance. Excluded paths (`target/`, `node_modules/`, `.git/`) never
  sync back and are never indexed.

### W3.4: env and credential policy for toolchains

- Extend the env policy beyond `ANTHROPIC_API_KEY`: a toolchain may need
  `CARGO_NET_GIT_FETCH_WITH_CLI`, an `NPM_TOKEN`, etc. Make the allow/deny list
  per-run config, defaulting to a minimal allowlist (the receiver's secret-stripping
  is the safe default; recipe-based execution currently does not strip env, a
  pre-existing risk W3 must not inherit).

## Acceptance criteria

1. A Rust test imports a tiny buildable fixture (a hello-world `cargo` crate) via W0,
   materializes it to a temp dir, runs `cargo build` through the bridge, and asserts a
   `ProofReceipt` with exit code 0 and the build's stdout captured. Real toolchain,
   real process, real exit code.
2. The source/artifact boundary holds: after the build, `target/` exists on disk but
   no `target/` path is in the `DocTree` and no `File` node was created for a build
   artifact (re-indexing artifacts would make builds unusable; this proves it does
   not happen).
3. Sync-back works: a run that rewrites a source file (e.g. `cargo fmt`) is detected,
   the changed source file syncs back into the `DocTree`, and the W1 on-write hook
   fires so the code graph reflects the edit. The agent never called "reindex."
4. The same `RunPlan` runs identically against both backends: a test runs it through
   `LocalProcessSandbox` and (gated, `#[ignore]` for a live sidecar) through
   `OpenSandboxRuntime`, returning the same `ProofReceipt` shape. Isolation is a
   backend choice, not a rewrite.
5. Env policy strips secrets by default: a test asserts a sensitive env var is not
   present in the child process unless explicitly allowlisted for the run.
6. `cargo test -p` (the bridge crate) green; changed files clippy-clean; the bridge
   pulls in no server framework into the embedded minimal-binary path.

## Divergences and risks to surface (not bury)

- **Recipe-based execution does not strip env today.** `runtime_plan_from_recipe`
  (`head.rs:108`) sets `strip_env: Vec::new()` (grounding flagged this). W3 must not
  build on that path without fixing the env policy, or it leaks secrets into the
  toolchain child.
- **`put_files`/`get_files` are unwired.** The sandbox backend has the methods but the
  receiver never calls them to materialize a codebase. W3 is the first caller; expect
  to discover gaps in the sidecar's file-staging contract and surface them.
- **Long builds need streaming.** `run_proof` writes to temp files and reads on
  completion (25ms poll). A 5-minute `cargo build` gives no live output and no
  cancellation mid-run. Live output / cancellation is a named follow-up; W3 ships the
  blocking version first.
- **Materialize is a copy with a drift surface.** Under DocTree-primary, the
  materialized dir and the `DocTree` can drift if the toolchain edits while the engine
  also writes. W3's sync-back is the reconciliation; W6 (FUSE) removes the copy and
  the drift entirely. Until then, keep the materialize window scoped (one run, one
  sync-back) rather than a long-lived mirror.
- **Isolation strength is a product decision.** The `OpenSandboxRuntime` sidecar's
  actual isolation (container, VM, namespace) is the sidecar's concern, not the
  bridge's. W3 ships the trait and two backends; the deployment picks the sidecar's
  isolation tier. Containerized child processes vs a lighter sandbox is answered by
  "whatever backend the deployment configures," not hardcoded in the bridge.
