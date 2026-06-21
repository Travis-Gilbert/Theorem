# For a large greenfield multi-file frontend, author the shared contract (tokens, typed client, primitives, build-risk-concentrated components) by hand and typecheck it GREEN first, then fan out the leaf surfaces to parallel agents that only READ the foundation and CREATE disjoint files

**Kind:** method
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (harness-console: ~55-file Next.js console from two specs)`
**Domain tags:** orchestration, workflow, frontend, parallel-agents, build-coherence, harness-console

## Trigger

I had to build `apps/harness-console` — a ~55-file Next.js 15 console — from two specs in one push. The obvious move (fan every surface out to parallel agents at once) would have raced on the shared files every surface imports (the token CSS, `lib/harness` types/client, the shadcn-style primitives) and drifted on import paths and prop shapes, producing a tree that compiles in pieces but not together.

## Method (what worked)

1. **Author the shared contract solo, and make it green on disk before any fan-out.** I hand-wrote: the one token file (`globals.css`), `lib/harness` (the typed `HarnessClient` + deterministic fixtures), ~20 Radix/shadcn-style primitives, the depth layer, the shell, and — critically — the 4 highest-build-risk components myself (cosmos.gl graph, CodeMirror editor, the two-synced-Yjs-doc collaborative IDE, RetroUI). Ran `tsc --noEmit` → exit 0 before proceeding.
2. **Fan out only the leaf surfaces.** A workflow spawned 7 parallel agents for the 10 surfaces + onboarding. Each prompt forced two rules: (a) READ the actual foundation files for exact APIs — never work from my prose description; (b) CREATE only your own page/component files, touch nothing shared (no edits to `lib/`, `components/ui`, `globals.css`, configs). Disjoint file sets => no filesystem races.
3. **Let the compiler be the integration test.** After the fan-out, `tsc` + `next build` were the integration verifier; then an adversarial-review workflow (4 lenses + per-finding verification) caught the one real bug.

## Rule

- Concentrate build risk in components YOU author (anything touching WebGL, CRDTs, dynamic imports, SSR boundaries); delegate the mechanical leaf surfaces.
- Parallel agents must ground on real files via Read, not on a description, and must own disjoint paths. The shared contract must typecheck green BEFORE fan-out, or the parallelism manufactures integration bugs.
- Do not trust "each agent says it's done" — run `tsc`/`next build` yourself as the join, then an adversarial review pass for the bugs the compiler can't see.

## Evidence

- 7 agents produced 37 surface files; first integration after fan-out: `tsc` clean and `next build` green (15 routes), zero import/prop drift across agents.
- The review workflow's rate-limited lenses still surfaced exactly one real bug (a `useEffect` deps array including a per-render-new `skill.files`, which snapped the active file back on every keystroke) — the class of bug parallel authoring produces, caught by the join + review rather than by any single agent.
