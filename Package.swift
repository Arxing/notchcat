// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "NotchCat",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "NotchCatKit"),
        .executableTarget(name: "NotchCat", dependencies: ["NotchCatKit"]),
        .testTarget(name: "NotchCatKitTests", dependencies: ["NotchCatKit"]),
    ]
)
