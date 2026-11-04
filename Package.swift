// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NotchCat",
    platforms: [.macOS(.v14)],
    products: [
        // Loaded only inside `/usr/bin/perl` by PerlHelperLauncher; nothing in
        // the Swift app links against it.
        .library(name: "NotchCatMediaRemoteHelper", type: .dynamic, targets: ["NotchCatMediaRemoteHelper"]),
    ],
    targets: [
        .target(name: "NotchCatKit"),
        .executableTarget(name: "NotchCat", dependencies: ["NotchCatKit"]),
        .target(name: "NotchCatMediaRemoteHelper", linkerSettings: [.linkedFramework("CoreFoundation")]),
        .testTarget(name: "NotchCatKitTests", dependencies: ["NotchCatKit"]),
    ]
)
