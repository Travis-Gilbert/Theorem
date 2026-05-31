// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TheoremIOS",
    defaultLocalization: "en",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .executable(
            name: "TheoremIOS",
            targets: ["TheoremIOSApp"]
        ),
        .executable(
            name: "TheoremIOSSmoke",
            targets: ["TheoremIOSSmoke"]
        ),
        .library(
            name: "TheoremIOSCore",
            targets: ["TheoremIOSCore"]
        ),
    ],
    dependencies: [
        // Grape: SwiftUI force-directed graph (spec algo 1, force_graph). Ported
        // from the converged Swift lane so force_graph runs a live force sim
        // instead of a static Canvas seed.
        .package(url: "https://github.com/swiftgraphs/Grape", from: "1.1.0"),
    ],
    targets: [
        .target(
            name: "TheoremIOSCore",
            dependencies: [
                .product(name: "Grape", package: "Grape"),
            ],
            resources: [
                .process("Resources"),
            ]
        ),
        .executableTarget(
            name: "TheoremIOSApp",
            dependencies: ["TheoremIOSCore"]
        ),
        .executableTarget(
            name: "TheoremIOSSmoke",
            dependencies: ["TheoremIOSCore"]
        ),
    ]
)
