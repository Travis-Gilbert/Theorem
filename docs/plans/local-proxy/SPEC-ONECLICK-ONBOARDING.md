# Execution Spec: One-Click Onboarding (stranger to proxied Claude Code)

Date: 2026-06-27. Register: execution. Read `CONVENTIONS.md` first; its rules apply. Depends on `SPEC-LOCAL-PROXY-MVP.md`.

## Purpose

Make the path from a stranger landing on the Commonplace site to a working proxied Claude Code session as short as a database install. The standalone proxy spec ships the binary and the commands; this spec ships the onboarding around them: the site download and copy-paste, the account-to-substrate link, and a first-run that tells the user exactly what to do next. This is the momentum-catch surface. The target experience is the one Redis and Postgres set: copy one line, run one command, it works.

## Governing principle

Every step is copy-pasteable and every step reports its own success. The user never reads docs to recover from a step. The site detects the platform and hands the exact command. The binary, on first run, prints where the proxy is listening and how to point an agent at it. A doctor command confirms the whole chain. The account login issues the substrate identity and the local node picks it up, so signing in is the only identity step and the model credential is never part of it.

## What exists (do not rebuild)

- The standalone binary and commands from `SPEC-LOCAL-PROXY-MVP.md`: `brew install theorem`, `theorem harness`, `theorem proxy`, `theorem wrap claude`, prebuilt per-platform binaries, the `curl ... | sh` installer, CPU-only first-run default.
- The Commonplace site and sign-in. The harness identity onboarding that issues an API key and a tenant per account.
- The Commonplace-bundled sidecar and Connect Claude Code control (deliverable 6 of the proxy spec).

## Deliverables

### 1. Homebrew tap and formula
Build: a public Homebrew tap so `brew install theorem` resolves, with the formula pulling the right prebuilt binary per platform from GitHub releases and placing the `theorem` binary on PATH. The formula has no build-from-source step for the user.
Acceptance: `brew install theorem` on a clean macOS machine installs a runnable `theorem` binary. Verify on a machine with no Rust toolchain.

### 2. Site download and copy-paste block
Build: on the Commonplace sign-in and landing surface, a download control that detects the visitor's OS and offers the matching path: a copy-paste `brew install theorem` line with a copy button for macOS and Linux-with-brew, a `curl ... | sh` line for Linux without brew, and a direct binary download as the fallback. Below it, a copy-paste run line (`theorem harness`) and the connect line (`theorem wrap claude`), each with a copy button. The block shows the three steps in order: install, run, connect.
Acceptance: a visitor on macOS sees the brew line and the run and connect lines, each copyable in one click, and the three-step order is explicit. Verify by loading the page on macOS and on Linux and confirming the offered command matches.

### 3. Account-to-substrate link
Build: signing into Commonplace issues the harness key and tenant, and the local node picks them up without the user pasting a key. The mechanism is a `theorem login` that opens the browser to the account and writes the issued key to the local config, or the site hands a short-lived claim code the user pastes once into `theorem login`. The model credential is never requested or handled by this flow.
Acceptance: after `theorem login`, the local node authenticates to the Railway substrate as the signed-in tenant, and no Anthropic credential was requested at any point. Verify by signing in, running `theorem login`, and confirming substrate calls carry the tenant identity.

### 4. First-run guidance from the binary
Build: on first `theorem harness` or `theorem proxy`, the binary prints the listening address (`proxy live at http://localhost:PORT`), the one line to point Claude Code at it (the `ANTHROPIC_BASE_URL` export or the `theorem wrap claude` alternative), and a confirmation that it is running CPU-only with no downloads. The message is the next action, not a log dump.
Acceptance: a first run prints the address, the connect instruction, and the no-download confirmation, and a user who reads only that message can connect Claude Code. Verify by running on a clean machine and following only the printed text.

### 5. Doctor command
Build: `theorem doctor` checks the chain and reports each link: the proxy is listening, the substrate identity is present, an upstream model credential is reachable, and Claude Code's `ANTHROPIC_BASE_URL` points at the local proxy. Each check reports pass or the exact fix.
Acceptance: `theorem doctor` on a correctly connected machine reports all links green, and on a machine where Claude Code is not pointed at the proxy it reports that link red with the fix. Verify both states.

### 6. Connect control parity in Commonplace
Build: the Commonplace Connect Claude Code button performs the same connect step the CLI prints, writing `ANTHROPIC_BASE_URL` for Claude Code, so the bundled-app user and the standalone user reach the same connected state by different doors.
Acceptance: clicking Connect in Commonplace produces the same connected state that `theorem wrap claude` produces. Verify by connecting through the button and confirming a Claude Code session runs through the proxy.

## Build Table

| # | Current state | Feature | Location | Action | Desired outcome | Test |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | No brew path | Homebrew tap + formula pulling prebuilt binaries | release infra + tap repo | Build | `brew install theorem` works with no toolchain | [-] |
| 2 | Sign-in has no install affordance | Site download + three-step copy-paste block with OS detection | Commonplace site | Build | Visitor copies install, run, connect in three clicks | [-] |
| 3 | Key paste assumed | Account login issues key, local node picks it up, no model credential touched | Commonplace site + `theorem login` | Build | Sign in, `theorem login`, node is the signed-in tenant | [-] |
| 4 | Binary is silent on start | First-run prints address, connect line, no-download confirmation | CLI entry | Build | A user who reads only the start message can connect | [-] |
| 5 | No chain check | `theorem doctor` reports each link with the exact fix | CLI entry | Build | Doctor is green when connected, names the red link otherwise | [-] |
| 6 | Connect button not at parity | Commonplace Connect performs the CLI connect step | `commonplace-desktop-runtime` + app | Build | Button reaches the same connected state as the CLI | [-] |

Test legend: `[-]` open, `[x]` verified against the acceptance criterion, `[~]` deferred with a reason that names a real external blocker.

## Verify first

Confirm the exact CLI command strings against the repo entry (`theorem harness` / `theorem proxy` / `theorem wrap claude` / `theorem login` / `theorem doctor`), the harness identity issuance flow the site already uses, the prebuilt-binary release artifact names the Homebrew formula will reference, and the Claude Code settings path the Connect control and `theorem wrap claude` write. Build against the real strings.

## Acceptance for the whole flow

A stranger on the Commonplace site copies one install line, runs one command, runs the connect command, and has Claude Code working through the proxy, with `theorem doctor` green, and the only identity step was signing in. Verify the whole chain on a clean machine, offline for the model-independent steps.

## Where it lands

- Homebrew tap and formula: a tap repo plus the release CI from the proxy spec.
- Site download, copy-paste block, account-to-substrate link: the Commonplace site.
- `theorem login`, `theorem doctor`, first-run guidance: the CLI entry binary.
- Connect control parity: `commonplace-desktop-runtime` and the Commonplace app.
