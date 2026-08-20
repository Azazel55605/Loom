//! Loom desktop client.
//!
//! The window hosts the same React UI the web frontend uses; all privileged
//! work happens in `web-backend` over its HTTP API. The only native plugins are
//! persistence boundaries: public server configuration in Store and tokens in
//! the operating system credential store. See
//! `docs/adr/0010-desktop-secure-storage-and-network-config.md`.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_keyring_store::init())
        .run(tauri::generate_context!())
        .expect("error while running Loom desktop");
}
