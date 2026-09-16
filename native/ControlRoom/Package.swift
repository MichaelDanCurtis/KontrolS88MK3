// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ControlRoomNative",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "ControlRoomNative", targets: ["ControlRoomNative"]),
    ],
    targets: [
        .executableTarget(
            name: "ControlRoomNative",
            path: "Sources"
        ),
        .testTarget(
            name: "ControlRoomNativeTests",
            dependencies: ["ControlRoomNative"],
            path: "Tests"
        ),
    ]
)
