// swift-tools-version: 6.0
//
// TheoremKit — the core of the Theorem iOS app.
//
// Spec: ~/Downloads/SPEC-THEOREM-IOS-V1.md. Three targets:
//   - TheoremKit:        wire models, reprojection sliver, theme. Pure logic +
//                        platform-agnostic; builds and runs host-side, no Xcode.
//   - TheoremUI:         SwiftUI renderers + view model + switcher (depends on
//                        TheoremKit + Grape). Compile-checked host-side; rendered
//                        in the iOS app target.
//   - theoremkit-verify: the executable check harness (the test runner the
//                        standalone command-line-tools toolchain supports). When
//                        Xcode is present the checks graduate to an XCTest target.
//
// iOS-only surfaces (ActivityKit Dynamic Island, SFSafariViewController) live in
// the app target (apps/ios/TheoremApp), which is the only Xcode-gated piece.

import PackageDescription

let package = Package(
    name: "TheoremKit",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(name: "TheoremKit", targets: ["TheoremKit"]),
        .library(name: "TheoremUI", targets: ["TheoremUI"]),
        .executable(name: "theoremkit-verify", targets: ["TheoremKitVerify"]),
    ],
    dependencies: [
        // Grape: SwiftUI force-directed graph (spec algo 1, force_graph).
        .package(url: "https://github.com/swiftgraphs/Grape", from: "1.1.0"),
    ],
    targets: [
        .target(
            name: "TheoremKit",
            dependencies: [],
            resources: [
                // Berthold Akzidenz-Grotesk (display face). Travis supplied the
                // licensed files, so the spec's "fall back to body until the
                // license" gate is satisfied: Berthold ships as the display face.
                .process("Resources"),
            ],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .target(
            name: "TheoremUI",
            dependencies: [
                "TheoremKit",
                .product(name: "Grape", package: "Grape"),
            ],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .executableTarget(
            name: "TheoremKitVerify",
            dependencies: ["TheoremKit"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "TheoremKitTests",
            dependencies: ["TheoremKit"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
