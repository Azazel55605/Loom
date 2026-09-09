//! Loom desktop client.
//!
//! The window hosts the same React UI the web frontend uses; all privileged
//! work happens in `web-backend` over its HTTP API. The only native plugins are
//! persistence and transport boundaries: public server configuration in Store,
//! tokens in the operating system credential store, and native HTTP/WebSocket
//! clients. The HTTP plugin provides the explicit per-server invalid-certificate
//! opt-in; the WebSocket plugin does not yet expose an equivalent runtime
//! policy. See `docs/adr/0010-desktop-secure-storage-and-network-config.md`.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopPlatformInfo {
    os: &'static str,
    is_flatpak: bool,
}

/// The frontend needs distribution context as well as the compile-time OS:
/// Flatpak owns its update lifecycle even though it is still a Linux build.
#[tauri::command]
fn desktop_platform_info() -> DesktopPlatformInfo {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };

    DesktopPlatformInfo {
        os,
        is_flatpak: std::env::var_os("FLATPAK_ID").is_some(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_keyring_store::init());

    // The updater configuration exists only in tauri.windows.conf.json.
    // Registering the plugin elsewhere makes Tauri pass a null configuration
    // to its deserializer, which aborts application startup before the
    // check-only Linux/macOS update path can run.
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .invoke_handler(tauri::generate_handler![desktop_platform_info])
        .run(tauri::generate_context!())
        .expect("error while running Loom desktop");
}

#[cfg(test)]
mod tests {
    use super::desktop_platform_info;

    #[test]
    fn platform_detection_always_names_the_compiled_desktop_os() {
        let platform = desktop_platform_info();
        assert!(matches!(platform.os, "windows" | "macos" | "linux"));
    }
}
