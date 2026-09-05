// swift-tools-version:5.3
// The Swift side of tauri-plugin-headstate-refresh. Built and linked
// into the app's static library by the plugin's build.rs (via swift-rs);
// the Xcode project under src-mobile/gen/apple never references it
// directly. `BackgroundTasks` is a system framework and is autolinked
// from the `import`.

import PackageDescription

let package = Package(
  name: "tauri-plugin-headstate-refresh",
  platforms: [
    // The app's deployment target (gen/apple/project.yml).
    // BGTaskScheduler needs iOS 13.
    .iOS(.v14)
  ],
  products: [
    .library(
      name: "tauri-plugin-headstate-refresh",
      type: .static,
      targets: ["tauri-plugin-headstate-refresh"])
  ],
  dependencies: [
    // Copied here by build.rs from the tauri crate; see ios/.gitignore.
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-headstate-refresh",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
