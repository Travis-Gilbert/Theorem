# Theorem iOS

Native SwiftUI client scaffold for `SPEC-THEOREM-IOS-V1.md`.

This package now has both the headless SwiftPM path and a generated Xcode app target. The core UI, scene models, projection switcher, Swift-native reprojection, and Dynamic Island shell live in source here; the Xcode target in `App/` is the thin app wrapper for simulator runs, signing, and TestFlight.

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

## Xcode Simulator Check

Generate or refresh the local Xcode project from the tracked XcodeGen spec:

```bash
cd apps/theorem-ios/App
xcodegen generate --spec project.yml
```

Build the app target for a booted simulator:

```bash
xcodebuild \
  -project apps/theorem-ios/App/Theorem.xcodeproj \
  -scheme Theorem \
  -configuration Debug \
  -destination 'platform=iOS Simulator,id=DB173B44-F97F-4969-A8E6-D6CA9221DEA6' \
  -derivedDataPath /tmp/theorem-xcode-derived \
  CODE_SIGNING_ALLOWED=NO \
  build
```

Install, launch, and capture a screenshot:

```bash
xcrun simctl install DB173B44-F97F-4969-A8E6-D6CA9221DEA6 \
  /tmp/theorem-xcode-derived/Build/Products/Debug-iphonesimulator/Theorem.app
xcrun simctl launch DB173B44-F97F-4969-A8E6-D6CA9221DEA6 me.travisgilbert.theorem
sleep 3
xcrun simctl io DB173B44-F97F-4969-A8E6-D6CA9221DEA6 screenshot \
  /tmp/theorem-ios-xcode-regenerated.png
```

The app target is validated with Xcode 26.5 against the iOS 26.5 simulator. The first frame can be white while SwiftUI mounts; wait a few seconds before treating a screenshot as a visual smoke.

## Shipping Notes

The v1 app ships with Swift-native reprojection. A Rust UniFFI `.xcframework` remains a future parity or performance lane, not a TestFlight blocker.

Debug keeps signing disabled for local simulator builds. Release leaves signing enabled and automatic, so TestFlight requires an Apple Developer Team, provisioning, and App Store Connect setup before archive upload.

No production entitlements are configured yet. The current Dynamic Island is in-app chrome, not an ActivityKit Live Activity, so adding ActivityKit would be premature.

`App/ExportOptions.plist` is the internal TestFlight upload recipe. After a signed archive exists, upload with:

```bash
xcodebuild \
  -exportArchive \
  -archivePath /tmp/Theorem.xcarchive \
  -exportPath /tmp/Theorem-export \
  -exportOptionsPlist apps/theorem-ios/App/ExportOptions.plist \
  -allowProvisioningUpdates
```

For the full one-command finish (archive plus export plus upload), use `App/ship-testflight.sh` once you have an Apple Developer Team ID:

```bash
THEOREM_TEAM_ID=XXXXXXXXXX apps/theorem-ios/App/ship-testflight.sh
```

It fails loudly if the team or an auth session (signed-in Xcode account, or `THEOREM_ASC_*` API key vars) is missing. See `docs/plans/theorem-ios-v1/native-app-shipping.md` for the owner credential steps.
