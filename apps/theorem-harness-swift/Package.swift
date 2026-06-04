// swift-tools-version:5.9
import PackageDescription

// SwiftPM package for the Theorem harness Swift binding. The iOS app depends on
// this package and `import TheoremHarness`.
//
// The binary target is the Rust core compiled for iOS device + simulator
// (TheoremHarnessFFI.xcframework, produced by ./build-xcframework.sh). The source
// target is the UniFFI-generated Swift API, which imports the C module the
// xcframework vends.
let package = Package(
    name: "TheoremHarness",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "TheoremHarness", targets: ["TheoremHarness"])
    ],
    targets: [
        .binaryTarget(
            name: "theorem_harness_swiftFFI",
            path: "TheoremHarnessFFI.xcframework"
        ),
        .target(
            name: "TheoremHarness",
            dependencies: ["theorem_harness_swiftFFI"],
            path: "Sources/TheoremHarness"
        ),
    ]
)
