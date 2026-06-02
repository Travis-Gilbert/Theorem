# Native app shipping receipt

This is the #7 native app shipping slice: full Xcode target validation,
simulator runtime proof, the Rust reprojection bridge decision, and the
remaining TestFlight path.

## Proven locally

Environment:

- Xcode 26.5, build 17F42.
- iOS Simulator 26.5.
- Destination: iPhone 17 Pro, `DB173B44-F97F-4969-A8E6-D6CA9221DEA6`.
- Bundle ID: `me.travisgilbert.theorem`.

Source-of-truth project generation:

```bash
cd apps/theorem-ios/App
xcodegen generate --spec project.yml
```

Debug simulator build:

```bash
rm -rf /tmp/theorem-xcode-derived
xcodebuild \
  -project apps/theorem-ios/App/Theorem.xcodeproj \
  -scheme Theorem \
  -configuration Debug \
  -destination 'platform=iOS Simulator,id=DB173B44-F97F-4969-A8E6-D6CA9221DEA6' \
  -derivedDataPath /tmp/theorem-xcode-derived \
  CODE_SIGNING_ALLOWED=NO \
  build
```

Result: `** BUILD SUCCEEDED **`.

Runtime smoke:

```bash
xcrun simctl install DB173B44-F97F-4969-A8E6-D6CA9221DEA6 \
  /tmp/theorem-xcode-derived/Build/Products/Debug-iphonesimulator/Theorem.app
xcrun simctl launch DB173B44-F97F-4969-A8E6-D6CA9221DEA6 me.travisgilbert.theorem
sleep 3
xcrun simctl io DB173B44-F97F-4969-A8E6-D6CA9221DEA6 screenshot \
  /tmp/theorem-ios-xcode-regenerated.png
```

Result: install and launch succeeded, and the screenshot shows the native app
shell, graph, and bottom search island.

## Rust bridge decision

Swift-native reprojection is the v1 shipping path. `TheoremProjectionEngine`
already runs the mobile projection set on-device and preserves the honest-shape
rule for unavailable projections.

Keep UniFFI and the Rust `.xcframework` as a future parity/performance lane,
not a TestFlight blocker. Reopen the Rust bridge if one of these becomes true:

- Swift projection performance misses the interaction target on real devices.
- Mobile availability must be byte-aligned with Rust `scene-os-core`.
- The Rust projection catalog becomes the only acceptable source of projection
  truth for mobile.

## TestFlight path

Release signing is now left enabled in `apps/theorem-ios/App/project.yml`, while
Debug keeps signing disabled for local simulator builds.

Remaining external steps:

1. Assign the Apple Developer Team in the generated Xcode project or through
   XcodeGen settings.
2. Confirm the Bundle ID `me.travisgilbert.theorem` in the developer account.
3. Add production entitlements only when the product surface needs them. This is
   complete for the current surface: no production entitlements are configured,
   and the current Dynamic Island is in-app chrome, not an ActivityKit Live
   Activity.
4. Build a signed Release archive:

```bash
xcodebuild \
  -project apps/theorem-ios/App/Theorem.xcodeproj \
  -scheme Theorem \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -archivePath /tmp/Theorem.xcarchive \
  archive
```

5. Upload after archive validation with `apps/theorem-ios/App/ExportOptions.plist`,
   then start internal TestFlight before external review:

```bash
xcodebuild \
  -exportArchive \
  -archivePath /tmp/Theorem.xcarchive \
  -exportPath /tmp/Theorem-export \
  -exportOptionsPlist apps/theorem-ios/App/ExportOptions.plist \
  -allowProvisioningUpdates
```

## Current blockers

- Apple Developer Team and provisioning are not configured in the repo.
- App Store Connect metadata, privacy answers, and review notes still need owner
  choices.
- Actual App Store Connect upload still requires a signed archive and either an
  Xcode account session or App Store Connect API key flags.
- Live hosted search readiness is separate from native app packaging readiness.

## Owner credential steps and one-command ship (2026-06-02)

Re-verified at current HEAD: the Debug simulator build is green (`** BUILD SUCCEEDED **`). The Release archive fails on exactly one thing, the missing team:

```
error: Signing for "Theorem" requires a development team.
** ARCHIVE FAILED ** (exit 65)
```

This machine has zero code-signing identities, no provisioning profiles, and no signed-in Xcode account. So there is no "deploy now, credentials later": a signed archive needs the team, and the upload needs an authenticated session. Both are owner-only.

What only the account owner can provide:

1. Team ID: developer.apple.com -> Membership -> Team ID (10 chars). Export as `THEOREM_TEAM_ID`.
2. ONE auth path:
   - A) Sign into Xcode once (Xcode -> Settings -> Accounts -> (+) -> the Developer Apple ID); then `THEOREM_TEAM_ID` alone is enough.
   - B) App Store Connect API key for headless upload: set `THEOREM_ASC_KEY_ID`, `THEOREM_ASC_ISSUER_ID`, and `THEOREM_ASC_KEY_PATH` (the `AuthKey_XXXX.p8`).
3. One-time: create the app record for `me.travisgilbert.theorem` in App Store Connect (My Apps -> +), or the first upload is rejected.

Then the one-command finish (archive plus export plus upload to internal TestFlight) is `apps/theorem-ios/App/ship-testflight.sh`:

```bash
THEOREM_TEAM_ID=XXXXXXXXXX apps/theorem-ios/App/ship-testflight.sh
```

The script regenerates the project, archives Release with `DEVELOPMENT_TEAM` and `-allowProvisioningUpdates`, and uploads via `ExportOptions.plist` (already app-store-connect, internal-testing-only). Pass `-a` to archive-only (validate signing without uploading). It fails loudly with the exact missing-credential message rather than producing an unsigned or fake artifact.
