// swift-tools-version:5.3
// The Swift side of tauri-plugin-headstate-keys. Built and linked into
// the app's static library by the plugin's build.rs (via swift-rs); the
// Xcode project under src-mobile/gen/apple never references it directly.

import PackageDescription

let package = Package(
  name: "tauri-plugin-headstate-keys",
  platforms: [
    // The app's deployment target (gen/apple/project.yml). CryptoKit's
    // Secure Enclave P-256 key needs iOS 13; ML-DSA-65 is gated on
    // iOS 26 at the call site with `#available`.
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-headstate-keys",
      type: .static,
      targets: ["tauri-plugin-headstate-keys"])
  ],
  dependencies: [
    // Copied here by build.rs from the tauri crate; see ios/.gitignore.
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-headstate-keys",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
