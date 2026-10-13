// swift-tools-version:6.0
// Squib on iOS, best effort (plan phase 7, ADR-065).
//
// SquibCore: the UniFFI Swift bindings to the Rust core. Generated, not committed:
// scripts/build-ios-core.sh (macOS) or scripts/swift-linux-test.sh (Linux container).
// SquibKit: the par timer, the engine's adapter contract in plain Swift, with the clock
// and the cue output injected so it is tested without audio hardware.
//
// The native library comes from an XCFramework on Apple platforms, and from the Linux
// build of the core (a system library) when the package is tested on Linux.
import PackageDescription

#if os(Linux)
let ffi: Target = .systemLibrary(name: "squib_mobileFFI", path: "Generated/linux")
#else
let ffi: Target = .binaryTarget(name: "squib_mobileFFI", path: "Generated/SquibCoreFFI.xcframework")
#endif

let package = Package(
    name: "SquibKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "SquibKit", targets: ["SquibKit"]),
    ],
    targets: [
        ffi,
        .target(name: "SquibCore", dependencies: ["squib_mobileFFI"]),
        .target(name: "SquibKit", dependencies: ["SquibCore"]),
        .testTarget(name: "SquibKitTests", dependencies: ["SquibKit"]),
    ],
    // The generated bindings predate strict Swift 6 concurrency checking.
    swiftLanguageModes: [.v5]
)
