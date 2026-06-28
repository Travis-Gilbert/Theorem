# Homebrew tap for theorem-proxy

This is the brew-tap half of roadmap C.1. The from-source path already works today
(`apps/theorem-proxy/scripts/install.sh` -> `cargo install`); a tap makes it
`brew install theorem-proxy`.

## What's automated (in this repo)

- **`.github/workflows/release-proxy.yml`** builds the binary for macOS arm64, macOS x64,
  and Linux x64 and attaches them (plus `.sha256`) to a GitHub release, on a `proxy-v*`
  tag push.
- **`theorem-proxy.rb`** is the formula that installs those release binaries.

## What needs you (GitHub account actions I can't do)

1. **Tag a release** so the binaries get built and published:
   ```sh
   git tag proxy-v0.1.0 && git push origin proxy-v0.1.0
   ```
   Watch the `release-proxy` workflow; it creates the `proxy-v0.1.0` release with three
   binary assets + their `.sha256`.

2. **Create the tap repo** under your account, named exactly `homebrew-theorem`
   (Homebrew requires the `homebrew-` prefix): `github.com/Travis-Gilbert/homebrew-theorem`.

3. **Put the formula in the tap.** Copy `theorem-proxy.rb` into the tap repo's
   `Formula/theorem-proxy.rb`, and paste the three `sha256` values from the release's
   `.sha256` assets into the `REPLACE_WITH_*` slots (and bump `version`/`url` to match the
   tag). Commit + push.

4. **Install:**
   ```sh
   brew tap Travis-Gilbert/theorem
   brew install theorem-proxy
   theorem-proxy wrap -- claude
   ```

## Caveat: the Theorem repo is private

The release **assets** (the binaries) must be reachable for `brew install` to download
them. A release on a private repo needs auth to fetch. Easiest options:
- Make just the `proxy-v*` releases public (the source stays private), or
- Move the release artifacts to a public repo (e.g. the `homebrew-theorem` tap's own
  releases) and point the formula's `url` there.

Until then, `scripts/install.sh` (build-from-source) is the working local install.
