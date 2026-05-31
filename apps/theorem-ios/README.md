# Theorem iOS

Native SwiftUI client scaffold for `SPEC-THEOREM-IOS-V1.md`.

This package is intentionally Xcode-light for the first slice: it builds with `swift build` and keeps the core UI, scene models, projection switcher, and Dynamic Island shell in source. After full Xcode is installed, wrap this package in an iOS app project for simulator runs, signing, TestFlight, and the Rust `.xcframework` bridge.

## Current Surface

- `TheoremIOSCore` mirrors the `ScenePackageV2` wire contract.
- The projection engine implements the v1 mobile set: `force_graph`, `radial_rings`, `tree_layout`, and `fractal_expansion`.
- Projection availability follows the honest-shape rule: tree rejects cycles and multi-parent graphs instead of fabricating hierarchy.
- The SwiftUI shell has the persistent Dynamic Island, paged five-surface IA, projection picker, node readout, and sample scene rendering.
- `SafariReaderView` is compiled only on iOS and is the host handoff seam for live page reads.

## Font Gate

Commercial Akzidenz-Grotesk font files are not tracked here. For local visual testing after license confirmation, place fonts in `Sources/TheoremIOSCore/Resources/Fonts/`. The app reads font tokens by family name and safely falls back if the font is absent.

IBM Plex Sans SemiCondensed remains the safe bundled default once font assets are added.

## Local Check

```bash
swift build --package-path apps/theorem-ios
swift run --package-path apps/theorem-ios TheoremIOSSmoke
```

Full iOS archive and simulator validation require full Xcode, not just Command Line Tools.
