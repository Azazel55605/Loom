//! Loom mobile client.
//!
//! The window hosts the same React UI the web frontend uses; all privileged
//! work happens in `web-backend` over its HTTP API, so this shell exposes
//! persistence and native HTTP transport plugins plus one window-decoration
//! command, and no privileged app-specific commands.
//! Public server configuration is persisted by Store; authentication tokens
//! are encrypted by Stronghold. See
//! `docs/adr/0010-desktop-secure-storage-and-network-config.md`. The single
//! command, `set_immersive_mode`, hides the Android system bars while kiosk
//! presentation is active — see `immersive` and ADR 0036.

mod immersive;
mod launcher;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            immersive::set_immersive_mode,
            launcher::is_acting_as_launcher,
            launcher::open_home_app_settings
        ])
        .setup(|app| {
            let salt_path = app
                .path()
                .app_local_data_dir()
                .expect("mobile app-local data directory is unavailable")
                .join("stronghold-salt.txt");
            app.handle()
                .plugin(tauri_plugin_stronghold::Builder::with_argon2(&salt_path).build())?;
            Ok(())
        })
        .plugin(tauri_plugin_store::Builder::default().build())
        .build(tauri::generate_context!())
        .expect("error while building Loom mobile")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Resumed) {
                restore_missing_webviews(app);
            }
        });
}

/// Rebuilds the configured webview when an Activity comes back without one.
///
/// Android may destroy the Activity while keeping this process alive — which is
/// routine for a home app, whose Activity is started and torn down far more
/// often than an ordinary app's. The Activity that comes back then has no
/// webview attached and no frontend is ever loaded, so the app draws an empty
/// window and no JavaScript runs at all (tauri-apps/tauri#15671). Rebuilding
/// from the same window config `setup` would have used restores the UI.
///
/// A no-op whenever a webview is present, which is every ordinary resume.
fn restore_missing_webviews<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if !app.webview_windows().is_empty() {
        return;
    }
    for window in app.config().app.windows.clone() {
        let label = window.label.clone();
        match tauri::WebviewWindowBuilder::from_config(app, &window)
            .and_then(|builder| builder.build())
        {
            Ok(_) => {}
            // Reported rather than fatal: the alternative to an empty window is
            // not a crash on the user's wall-mounted display.
            Err(error) => eprintln!("could not restore the webview for {label}: {error}"),
        }
    }
}
