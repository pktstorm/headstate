//! Standard Tauri plugin build script. `COMMANDS` names the native
//! commands so the permission files exist in the shape Tauri expects;
//! nothing in the webview is ever granted them, because every call
//! into this plugin comes from Rust (`run_mobile_plugin`), which does
//! not pass through the ACL. The frontend cannot start or complete a
//! background window.

const COMMANDS: &[&str] = &["register", "complete"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
