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

## Apple gate checklist: first ship to a physical device (2026-06-03)

Learned shipping build 1/2 of Theorem iOS onto a physical iPhone 17 Pro. Each item below was a distinct, separately-discovered blocker. After the first build most are done; this is the "do not rediscover it" list. The substrate memory copy is `doc_671af9eea614642f`.

Account and signing (owner, one-time):

1. Apple Developer Program enrollment must be ACTIVE and the Apple Developer Program License Agreement ACCEPTED (developer.apple.com, Account). An unaccepted agreement keeps the team invisible everywhere (empty `IDEProvisioningTeams`, zero signing certs) with no obvious error message.
2. Sign into Xcode itself: Xcode, Settings, Accounts, (+), Apple ID. This is NOT the same as logging into the developer website or App Store Connect in a browser; only the in-Xcode account gives the machine a signing identity. Verify with `defaults read com.apple.dt.Xcode DVTDeveloperAccountManagerAppleIDLists` (a non-empty list means the account is added).
3. Team ID: developer.apple.com, Membership, Team ID (10-char alphanumeric). Pass as `DEVELOPMENT_TEAM` to xcodebuild. Without it: `Signing for "Theorem" requires a development team` (exit 65). xcodebuild will not infer it even with an account signed in.
4. Register the device UDID: developer.apple.com, Devices, (+). Get the real UDID (modern 24-hex-plus-dash form, e.g. `00008150-...`) via `xcrun devicectl device info details --device <id> | grep -i udid` (the devicectl `identifier` is NOT the UDID). Until a device is registered, automatic signing cannot create the development provisioning profile the archive embeds ("Your team has no devices from which to generate a provisioning profile"); network pairing alone is not enough.

Build content:

5. An app icon is required, or App Store Connect rejects the upload (90713 missing `CFBundleIconName`, 90022 missing 120x120). Use `Assets.xcassets` with a single-size 1024 `AppIcon` (opaque, no alpha; the store rejects alpha) plus `ASSETCATALOG_COMPILER_APPICON_NAME: AppIcon` in `project.yml`. Generate the PNG with Swift/Node CoreGraphics (gen-AI icons turn to mush at 60px). The shipped icon is a generated placeholder (constellation mark); replace before any public release.
6. Export compliance is per build (`internalBuildState=MISSING_EXPORT_COMPLIANCE` blocks distribution). Either set `INFOPLIST_KEY_ITSAppUsesNonExemptEncryption: NO` in `project.yml` to auto-answer every build, or clear it per build via the App Store Connect API (`PATCH /v1/builds/{id}`, attribute `usesNonExemptEncryption=false`).

Signing model:

7. The archive is signed with an Apple DEVELOPMENT identity (which needs the registered device). The export step (`-exportArchive`, `method=app-store-connect`) re-signs for App Store DISTRIBUTION, which is device-free. Both are auto-created by `-allowProvisioningUpdates` once account, team, device, and agreement are in place.

TestFlight (the slow, opaque path):

8. Internal testers must be added via the App Store Connect UI user-picker (checkbox existing account users), NOT by typing an email and NOT via `POST /v1/betaTesters`. Note: `inviteType=EMAIL` is reported for internal testers too; it is not the internal-vs-external discriminator. Ignore "invitation has been revoked or is invalid" emails; internal testers do not redeem links, and a deleted tester's email always errors.
9. First-build backend lag is real and opaque: a build can read `IN_BETA_TESTING` with everything green (VALID, not expired, minOS below the device OS, group `hasAccessToAllBuilds=true`, tester present) and still show "No Builds Available" and never appear in the TestFlight app for a long time. It is not forceable.

The deterministic bypass (when TestFlight stalls):

10. With a dev-signed archive plus a registered device plus Developer Mode ON (iPhone, Settings, Privacy and Security, Developer Mode; requires a restart; "Developer Mode disabled" surfaces as `CoreDeviceError 10005`), install straight to the phone:

```bash
xcrun devicectl device install app --device <DEVICE_ID> /tmp/Theorem.xcarchive/Products/Applications/Theorem.app
xcrun devicectl device process launch --device <DEVICE_ID> me.travisgilbert.theorem
```

On a paid account the development profile is valid for about a year (no 7-day expiry), so this is a durable way to keep the app on the phone, decoupled from TestFlight.

App Store Connect API (status checks plus headless uploads): needs the Key ID (in the `AuthKey_<KEYID>.p8` filename), the Issuer ID (Users and Access, Integrations; a UUID, not in the `.p8`), and the `.p8`. There was no Python JWT library on this Mac, so mint the ES256 JWT with Node's built-in crypto: `crypto.sign('sha256', input, {key, dsaEncoding: 'ieee-p1363'})` (raw r-or-s, not DER); Node also has built-in `fetch`. Keep the `.p8` out of git. macOS bash 3.2 with `set -u`: expand a possibly-empty array as `${arr[@]+"${arr[@]}"}` or it throws "unbound variable".
