// swift-tools-version: 6.0
//
// TheoremKit — the headlessly-testable core of the Theorem iOS app.
//
// Spec: ~/Downloads/SPEC-THEOREM-IOS-V1.md. This package holds the wire models,
// the on-device reprojection sliver (behind a protocol), the role-based theme,
// and the SwiftUI renderers. It builds on the macOS host (no Xcode project, no
// simulator) so the foundation is verifiable; iOS-only surfaces (ActivityKit
// Dynamic Island, SFSafariViewController) are `#if os(iOS)`-guarded so the
// package still compiles for the host.
//
// Verification: `swift run theoremkit-verify` runs the executable check harness
// (the only test runner the standalone command-line-tools toolchain supports —
// the `Testing`/`XCTest` modules ship with full Xcode). When the Xcode app
// target (apps/ios/TheoremApp) is added, the checks graduate to an XCTest/Swift
// Testing target run via XcodeBuildMCP.

import PackageDescription

let package = Package(
    name: "TheoremKit",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(name: "TheoremKit", targets: ["TheoremKit"]),
        .executable(name: "theoremkit-verify", targets: ["TheoremKitVerify"]),
    ],
    dependencies: [
        // Grape (force-directed graph) is added when the force_graph renderer
        // lands (B2.1) so the B0 foundation stays dependency-light and fast to
        // build. https://github.com/swiftgraphs/Grape from: 1.1.0
    ],
    targets: [
        .target(
            name: "TheoremKit",
            dependencies: [],
            resources: [
                // Berthold Akzidenz-Grotesk (display face). Travis supplied the
                // licensed files, so the spec's "fall back to body until the
                // license" gate is satisfied: Berthold ships as the display
                // face. Registered at runtime from Bundle.module (see Theme).
                .process("Resources"),
            ],
            swiftSettings: [
                .swiftLanguageMode(.v6),
            ]
        ),
        .executableTarget(
            name: "TheoremKitVerify",
            dependencies: ["TheoremKit"],
            swiftSettings: [
                .swiftLanguageMode(.v6),
            ]
        ),
    ]
)
