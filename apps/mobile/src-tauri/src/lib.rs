//! Loom mobile client.
//!
//! The window hosts the same React UI the web frontend uses; all privileged
//! work happens in `web-backend` over its HTTP API, so this shell deliberately
//! exposes no custom commands yet. See `docs/ARCHITECTURE.md`.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Loom mobile");
}
