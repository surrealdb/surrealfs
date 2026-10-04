// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SurrealFSMenu",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(
            name: "SurrealFSMenu",
            targets: ["SurrealFSMenu"]
        )
    ],
    dependencies: [],
    targets: [
        .executableTarget(
            name: "SurrealFSMenu",
            dependencies: [],
            path: "Sources/SurrealFSMenu"
        ),
        .testTarget(
            name: "SurrealFSMenuTests",
            dependencies: ["SurrealFSMenu"],
            path: "Tests/SurrealFSMenuTests"
        )
    ]
)
