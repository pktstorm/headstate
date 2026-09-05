// The desktop-host binary exists so `cargo check`, `cargo test`, and `cargo
// clippy` have a `main` to build; on iOS and Android the platform project
// links the static library and calls the mobile entry point instead.
fn main() {
    headstate_companion_lib::run()
}
