// swift-tools-version: 6.0
import PackageDescription

// Run ./build.sh first: it builds the Rust bridge and generates
// CarbideFFI.xcframework and Sources/CarbideCore/carbide_bridge.swift.
let package = Package(
    name: "CarbideSilica",
    platforms: [.macOS(.v15)],
    targets: [
        .binaryTarget(name: "CarbideFFI", path: "CarbideFFI.xcframework"),
        .target(
            name: "CarbideCore",
            dependencies: ["CarbideFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "CarbideSilica",
            dependencies: ["CarbideCore"],
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
            ]
        ),
    ]
)
