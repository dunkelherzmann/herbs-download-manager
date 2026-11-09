// swift-tools-version:6.0
// macOS app: Swift 6 + AppKit over the Rust core (core/), called through Swift bindings that UniFFI generates
// from ffi/. Build with scripts/build-macos.sh, which builds the Rust library and the bindings first.
import PackageDescription

let rustLib = Context.packageDirectory + "/.build/rust"

let package = Package(
    name: "HerbsDownloadManager",
    platforms: [.macOS(.v15)],
    products: [
        .executable(name: "HerbsDownloadManager", targets: ["HerbsDownloadManager"]),
    ],
    targets: [
        // Generated: the C header and module map of the Rust library.
        .systemLibrary(name: "hd_ffiFFI", path: "macos/Sources/hd_ffiFFI"),
        // Generated: the Swift side of the bindings.
        .target(
            name: "HDCore",
            dependencies: ["hd_ffiFFI"],
            path: "macos/Sources/HDCore",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-L", rustLib]), .linkedLibrary("hd_ffi"),
                // The HTTP client reads the system proxy settings.
                .linkedFramework("SystemConfiguration"), .linkedFramework("CoreFoundation"), .linkedFramework("Security"),
            ]
        ),
        .executableTarget(
            name: "HerbsDownloadManager",
            dependencies: ["HDCore"],
            path: "macos/Sources/HerbsDownloadManager",
            linkerSettings: [.linkedFramework("AppKit")]
        ),
    ]
)
