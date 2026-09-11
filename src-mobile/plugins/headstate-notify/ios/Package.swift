// swift-tools-version:5.3
// The Swift side of tauri-plugin-headstate-notify. Built and linked into
// the app's static library by the plugin's build.rs (via swift-rs); the
// Xcode project under src-mobile/gen/apple never references it directly.
// `UserNotifications` is a system framework and is autolinked from the
// `import`.

import PackageDescription

let package = Package(
  name: "tauri-plugin-headstate-notify",
  platforms: [
    // The app's deployment target (gen/apple/project.yml), matching
    // tauri-plugin-headstate-refresh. UNUserNotificationCenter needs
    // iOS 10, so this is the app's floor rather than the API's.
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-headstate-notify",
      type: .static,
      targets: ["tauri-plugin-headstate-notify"])
  ],
  dependencies: [
    // Copied here by build.rs from the tauri crate; see ios/.gitignore.
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-headstate-notify",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
