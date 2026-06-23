# `railway up` silently drops gitignore negated-includes (`!/dist/x`), so an `include_str!`'d committed asset vanishes from the Docker build context — deploy GitHub-connected, which builds the git tree

**Kind:** gotcha
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (deploy theorem-gateway / SceneOS add-on)`
**Domain tags:** railway, deploy, monorepo, gitignore, docker-build-context, include_str, scene-os-web

## Trigger

Deploying `theorem-gateway` (new crate) to Railway via `railway up --detach` (local upload, chosen to avoid committing a co-agent's shared crate). The Docker build failed twice at the same spot:

```
error: couldn't read `/app/rustyredcore_THG/crates/scene-os-web/src/../web/dist/scene-os.bundle.js`: No such file or directory (os error 2)
  --> scene-os-web/src/lib.rs:36   const SCENE_BUNDLE: &str = include_str!("../web/dist/scene-os.bundle.js");
```

The bundle is `include_str!`'d into the Rust crate at compile time, so it MUST be in the build context. It looked present by every git check:
- `git ls-files --error-unmatch <bundle>` -> TRACKED
- `git check-ignore -v <bundle>` -> NOT ignored
- on disk, 170 KB, correct content

The crate's `web/.gitignore` was `/dist/*` then `!/dist/scene-os.bundle.js` (ignore byproducts, keep the bundle). Git honors that negation (hence "not ignored" + tracked). **`railway up`'s uploader does not** — it applied `/dist/*` and dropped the negated re-include, so the bundle was missing from the uploaded tarball / Docker context even though git tracks it. Editing the `.gitignore` to ignore byproducts by extension instead made `railway up` STILL drop it on retry (the uploader appears to exclude `dist/`-shaped paths more aggressively than git does).

Fix that worked first try: connect the service to the GitHub repo (`connect_service_source` -> `owner/repo@main`) and deploy from there. Railway clones the git tree, where the bundle is a tracked file, so the Docker `COPY` + `include_str!` both see it. Build succeeded, `/healthz` + a live `sceneForInput` round-trip both green.

## Rule

When a Railway service's Docker build `COPY`s an asset that is committed-but-gitignore-negated (the `/dir/*` + `!/dir/keep` pattern, common for committed build bundles embedded via `include_str!` / `include_bytes!`), do NOT deploy with `railway up` (local upload): its uploader does not honor the negated re-include and silently ships a context missing that file, so the build dies on a confusing "No such file" for a file git clearly tracks. Deploy GitHub-connected instead (`connect_service_source` then let Railway build the cloned git tree, which contains all tracked files) — or, if you must use `railway up`, verify the asset is actually in the upload, not just tracked by git. `git check-ignore` saying "not ignored" is NOT sufficient evidence that `railway up` will ship it.

## Evidence

- Two `railway up` deploys (ids `6fc42dee`, `15ef5e5b`) FAILED at `cargo build` with the missing-bundle `include_str!` error; both showed `-` for commit (local upload, no commit).
- `git check-ignore` returned non-zero (not ignored) and `git ls-files` confirmed tracked, yet the builder's `/app/.../web/dist/scene-os.bundle.js` did not exist.
- Switching to GitHub source (`connect_service_source theorem-gateway -> Travis-Gilbert/Theorem@main`) auto-triggered deploy `e83579d1` from commit `d8b831f8` -> SUCCESS; the build log shows `COPY rustyredcore_THG/crates` then a clean `cargo build --release` then `image push`.
- Live proof post-deploy: `/healthz` 200 `ok`; `search` returned `{"data":{"search":[]}}` (honest-empty over live theorem-grpc, proving the private-network dial); `sceneForInput` returned a real `SceneRef`; `GET /scene/{id}` served the bundle (markers `scene-annotation-model`, `__SCENE_PACKAGE__`).
- Adjacent lever that mattered: monorepo Dockerfile-at-subpath needs `RAILWAY_DOCKERFILE_PATH=apps/theorem-gateway/Dockerfile` as a service variable (the `serviceInstanceUpdate { builder: DOCKERFILE }` API mutation errored with "Problem processing request"; the env var is the reliable way to force the Dockerfile builder + path).
