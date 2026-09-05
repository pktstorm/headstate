//! Headstate Companion: the phone half of the mobile companion design
//! (docs/superpowers/specs/2026-09-05-mobile-companion-design.md).
//!
//! This crate is a thin client. It owns the TLS session and the device
//! keys and forwards every command to a paired desktop; it never holds a
//! GitHub token and never talks to GitHub. The modules the spec names --
//! `keys`, `client`, `pairing`, `events` -- arrive with #514; what is here
//! is the shell those plug into, plus `discovery`, which finds the paired
//! desktop on the LAN when its stored address has gone stale.

pub mod discovery;

pub mod background;

/// The shared frontend in `src/` is built with `VITE_TARGET=mobile`; its
/// `transport.ts` picks the remote transport on that value. Until #514
/// lands the mobile transport throws at import, by design, rather than
/// silently talking to a Rust process that is not there.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Same logging shape as the desktop: a GUI app has no stderr, and
        // "it would not pair" is uninvestigable without a file to ask for.
        // Never log a private key, a pairing token, or a repository owner
        // -- see CONTRIBUTING and check-privacy.sh.
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        // Opportunistic background refresh (#516): the OS-granted window
        // on each platform, running whatever `background::install` put
        // in state. On a desktop host it registers an inert scheduler.
        .plugin(tauri_plugin_headstate_refresh::init())
        .setup(|app| {
            background::install(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running Headstate Companion");
}

#[cfg(test)]
mod tests {
    /// The resolved dependency graph, for ALL targets: Cargo.lock lists
    /// every crate any platform would build, so this is stricter than a
    /// per-target `cargo tree`.
    const LOCK: &str = include_str!("../Cargo.lock");

    fn locked_crates() -> Vec<&'static str> {
        let names: Vec<&str> = LOCK
            .lines()
            .filter_map(|l| l.strip_prefix("name = \""))
            .map(|l| l.trim_end_matches('"'))
            .collect();
        assert!(!names.is_empty(), "Cargo.lock parsed to no packages");
        names
    }

    /// Crates the desktop needs and the phone must never link. The spec
    /// makes this the reason the companion is a separate crate at all:
    /// the phone has no GitHub token, no SQLite ledger, and nothing to
    /// clean. A path dependency on `headstate_lib`, or a copy-pasted
    /// dependency block, would drag these in without any code change
    /// noticing -- so the lockfile itself is asserted.
    #[test]
    fn no_desktop_only_crates_in_lock() {
        let names = locked_crates();
        for forbidden in [
            "octocrab",
            "rusqlite",
            "libsqlite3-sys",
            "bollard",
            "git2",
            "libgit2-sys",
            "headstate",
        ] {
            assert!(
                !names.contains(&forbidden),
                "{forbidden} is in src-mobile/Cargo.lock; the companion must not link it"
            );
        }
    }

    /// The TLS stack must sit on the aws-lc-rs provider: it is the only
    /// one with X25519MLKEM768. The other half of that rule -- no `ring`
    /// beside it -- is NOT asserted here, because the lockfile is
    /// all-targets and `ring` is resolved for platforms this crate never
    /// builds (it is absent from `cargo tree` on the host, iOS, and
    /// Android). `deny.toml` bans it on the two phone targets instead.
    #[test]
    fn tls_is_on_aws_lc_rs() {
        assert!(
            locked_crates().contains(&"aws-lc-rs"),
            "aws-lc-rs missing from the lock"
        );
    }

    /// The background refresh plugin is registered and given a
    /// refresher; without `install` a window would find nothing to run.
    #[test]
    fn the_background_refresh_is_wired() {
        let src = include_str!("lib.rs");
        assert!(src.contains(".plugin(tauri_plugin_headstate_refresh::init())"));
        assert!(src.contains("background::install(app.handle())"));
    }
}
