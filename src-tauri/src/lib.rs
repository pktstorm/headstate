pub mod auth;
pub mod commands;
pub mod github;
pub mod poll;
pub mod store;

use commands::{AuthState, GhClient};
use github::client::GitHubClient;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_cached,
            commands::refresh_now,
            commands::get_stats,
            commands::get_auth_state,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // `read_token` shells out via `std::process::Command`, which
            // blocks. `setup` runs as a plain synchronous closure before the
            // async runtime takes over this task, so calling it directly
            // here is safe -- it is not blocking a tokio worker.
            let (auth_state, gh_client) = match auth::read_token() {
                Ok(token) => match auth::build_client(&token) {
                    Ok(octocrab) => {
                        let client = Arc::new(GitHubClient::new(octocrab));
                        (
                            AuthState {
                                ok: true,
                                message: String::new(),
                            },
                            Some(client),
                        )
                    }
                    Err(e) => (
                        AuthState {
                            ok: false,
                            message: e.to_string(),
                        },
                        None,
                    ),
                },
                // `AuthError`'s Display messages (including
                // `GhNotLoggedIn`'s, which comes verbatim from `gh`'s own
                // stderr) are already display-ready prose for a first-run
                // screen -- never re-wrapped, never containing a token.
                Err(e) => (
                    AuthState {
                        ok: false,
                        message: e.to_string(),
                    },
                    None,
                ),
            };

            app.manage(auth_state);
            app.manage(GhClient(gh_client.clone()));

            // Only poll GitHub if we actually have a client; there is
            // nothing to fetch without one, and this task is the only
            // caller of GitHub in the whole app.
            if let Some(client) = gh_client {
                let focused = Arc::new(AtomicBool::new(true));
                poll::spawn(handle, client, focused);
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
