// swift-tools-version: 6.0
// Run ../scripts/build-core.sh first: it produces Frameworks/UniHubCoreFFI.xcframework
// and Sources/UniHubCore/UniHubCore.swift from the Rust core.
import PackageDescription

let package = Package(
    name: "UniHub",
    platforms: [.macOS(.v14)],
    targets: [
        .binaryTarget(name: "UniHubCoreFFI", path: "Frameworks/UniHubCoreFFI.xcframework"),
        // UniFFI-generated Swift wrapper around the Rust static library.
        .target(
            name: "UniHubCore",
            dependencies: ["UniHubCoreFFI"],
            path: "Sources/UniHubCore",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
                // The core reads the official Hub's hub.db with the SQLite that ships with macOS.
                .linkedLibrary("sqlite3"),
            ]
        ),
        .executableTarget(name: "UniHub", dependencies: ["UniHubCore"], path: "Sources/UniHub"),
    ]
)
