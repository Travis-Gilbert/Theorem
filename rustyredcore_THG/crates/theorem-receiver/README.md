# theorem-receiver

The local half of the dispatch-queue (docs/plans/dispatch-queue/HANDOFF.md). It
replaces the rejected GitHub-Actions dispatcher: an outbound-only claim loop that
spawns the locally-installed `claude` / `codex` CLI in a mapped worktree, using
their existing subscription logins. Zero new credentials, zero inbound ports,
zero GitHub Actions / runners / PATs / stored OAuth tokens.

## What it does

1. Detects lanes at startup: `which claude`, `which codex`. Registers only what
   is present (a machine without `codex` never claims Codex-lane jobs).
2. Polls the cloud harness with `job_claim` on an interval (and immediately after
   any job completes). Outbound only.
3. On a claim, spawns the head as a child process in the repo's worktree:
   - Claude lane: `claude -p "<intent>" --permission-mode acceptEdits`
   - Codex lane:  `codex exec "<intent>"`
   - `ANTHROPIC_API_KEY` is stripped from the child environment (an API key
     silently wins precedence over the subscription login and bills metered).
4. Streams the child's output, captures the exit code + a stdout tail, then
   defensively calls `job_complete(failed, <receipt>)`. This is idempotent: if the
   head already self-completed (Done/Failed) it is a no-op; if the head exited
   WITHOUT completing, the job is closed Failed with the exit receipt.

It does NOT run the RustyRed engine locally: no vector index, no PPR, no BM25, no
embedders. Idle footprint is listener-scale.

## Run it (Option B: standalone binary)

```bash
cp crates/theorem-receiver/theorem-receiver.example.toml ./theorem-receiver.toml
# edit the worktree map + harness_url
THEOREM_HARNESS_TOKEN=<bearer> \
  cargo run -p theorem-receiver -- ./theorem-receiver.toml
```

Deploy via `docker run` with a restart policy or launchd. Kubernetes is ruled out.

## Embed it (Option A: a capability of the local RustyRed node)

The crate is a library (`theorem_receiver`) as well as a binary. To make the
receiver a capability of a node binary you already run, depend on the crate and
drive the loop directly:

```rust
use theorem_receiver::{config::ReceiverConfig, HarnessClient, run_loop};

let config = ReceiverConfig::load("theorem-receiver.toml")?;
let token = std::env::var("THEOREM_HARNESS_TOKEN")?;
let client = HarnessClient::new(config.harness_url.clone(), token, config.tenant_slug.clone())?;
run_loop(&config, &client)?; // or spawn on its own thread
```

## Billing and policy

- From 2026-06-15, `claude -p` on a subscription draws from the separate, finite
  monthly Agent SDK credit bucket. The receiver logs a per-job usage line so the
  draw is measurable.
- Solo use on the owner's own repos is sanctioned individual use. The moment a job
  belongs to another user it must execute on that user's own key; that is the
  shelved RunPod lane, never the personal subscription login. This receiver only
  claims repos present in its local worktree map.

## Named follow-ups (not in this slice)

- SSE wake on the jobs channel (gated on the tenant-scoped push fix in push.rs);
  until it lands, polling is the mechanism.
- Parallel dispatch for `capacity > 1` (the loop is currently sequential).
