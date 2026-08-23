//! Loom mobile client.
//!
//! The window hosts the same React UI the web frontend uses; all privileged
//! work happens in `web-backend` over its HTTP API, so this shell exposes only
//! persistence and native HTTP transport plugins, not app-specific commands.
//! Public server configuration is persisted by Store; authentication tokens
//! are encrypted by Stronghold. See
//! `docs/adr/0010-desktop-secure-storage-and-network-config.md`.

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
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
        .run(tauri::generate_context!())
        .expect("error while running Loom mobile");
}
