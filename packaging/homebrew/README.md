# Homebrew Tap

`Formula/rustyred.rb` is generated from `rustyred.rb.template` when a tagged
RustyRed release has published tarballs and SHA-256 sums.

Release operator flow:

```bash
TAG=rustyred-v0.1.0
./packaging/homebrew/render-formula.sh "$TAG" > Formula/rustyred.rb
brew audit --strict Formula/rustyred.rb
brew test Formula/rustyred.rb
```

Publish `Formula/rustyred.rb` to the tap repository after the GitHub release
assets exist.
