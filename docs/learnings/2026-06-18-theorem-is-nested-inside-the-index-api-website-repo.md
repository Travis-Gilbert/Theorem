# The Theorem repo is a nested git repo INSIDE the Index-API website repo — the Next.js `/experiments` tiles that consume the gateway live in the PARENT repo, not Theorem

**Kind:** gotcha
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (theorem-gateway SceneOS Deliverable D)`
**Domain tags:** repo-topology, cross-surface, index-api, website, theorem-gateway, nextjs

## Trigger

Building Deliverable D of the gateway spec (the website `/experiments` omni-bar tile that calls the gateway). The spec called it "a separate-repo consumer." It turned out the website is not far away at all: `git rev-parse --show-toplevel` from `Creative/Website` is its OWN repo (`git@github.com:Travis-Gilbert/Index-API.git`, package `travisgilbert-website`, Next.js 16), and `Creative/Website/Theorem` is a SEPARATE nested git repo (the Rust projection, remote `Travis-Gilbert/Theorem`). So the working directory I'd been in the whole session (`Theorem`) is a child directory of the website repo, and the `/experiments` route already existed at `Creative/Website/src/app/(main)/experiments/page.tsx`.

Practical consequence: gateway backend work lands in the `Theorem` repo; the consuming website tile lands in the `Index-API` repo one directory up. They are two repos, two remotes, two deploy targets, and a markdown link relative to the Theorem cwd cannot reach the website files (they're at `../src/...`).

## Rule

When a Theorem task mentions "the website", "the `/experiments` tiles", "the frontend", or "the commonplace workbench", the target is the PARENT repo: `Creative/Website` (the `Index-API` repo, `travisgilbert-website`, Next.js 16 with the rough.js/`RoughBox` design system + `NEXT_PUBLIC_*` env convention + `@/*` -> `src/*` alias). It is NOT in the Theorem checkout (`apps/desktop` is Tauri, not the public site). Run `git rev-parse --show-toplevel` to confirm which repo a path belongs to before editing or committing, because the two repos nest and their git state / auto-commit hooks are independent. Frontend tiles that consume `theorem-gateway` are authored there and reach the gateway via `NEXT_PUBLIC_THEOREM_GATEWAY_URL` (raw `fetch` to `${url}/graphql`); nothing in Theorem materializes them.

## Evidence

- `git rev-parse --show-toplevel` in `Creative/Website` -> `.../Creative/Website` with remote `Index-API.git`; in `Creative/Website/Theorem` -> `.../Theorem` with remote `Theorem.git`. Two distinct repos, nested.
- The existing tile registry `Creative/Website/src/app/(main)/experiments/page.tsx` hand-curates `ExperimentEntry[]`; adding the SceneOS tile = a new entry + a new route `experiments/scene/{page.tsx, SceneOmnibar.tsx}` in the Index-API repo, while the gateway it calls lives in the Theorem repo.
- The website had zero prior gateway/GraphQL references (`grep -rn graphql src` empty before this), so the SceneOmnibar `fetch` to `${NEXT_PUBLIC_THEOREM_GATEWAY_URL}/graphql` is the first website->Theorem-gateway wire.
