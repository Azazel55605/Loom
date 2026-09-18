//! Android immersive mode: hiding and restoring the system bars.
//!
//! Tauri 2 has no API for this. Its window API — `setFullscreen` included — is
//! desktop-only, and hiding the status and navigation bars on Android is an
//! open request against the framework rather than something it exposes today.
//! No first-party plugin covers it either; the community plugins in this space
//! manage the status bar's appearance, not an immersive window. The documented
//! Android approach is therefore used directly: `WindowInsetsController` on
//! API 30 and above, and the deprecated `setSystemUiVisibility` flags below it.
//!
//! The calls are made over JNI from this crate rather than from Kotlin, because
//! `gen/android` is generated and untracked: code added to `MainActivity` would
//! have to be re-applied by the configure script on every machine, and would
//! still need a bridge for the frontend to toggle it per kiosk state.
//!
//! The JNI environment and the Activity come from Tauri's own webview handle
//! (`PlatformWebview::jni_handle`), which runs the closure on the thread that
//! owns the webview. Deliberately **not** from `ndk_context`: that crate's
//! context is installed by `ndk-glue`-style runtimes, nothing in Tauri's
//! Android stack installs it, and `ndk_context::android_context()` panics when
//! it was never initialized — on the UI thread, which takes the whole app down.
//!
//! Non-Android builds keep a no-op so the command exists on every platform the
//! mobile shell can be compiled for.

#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};
    use jni::JNIEnv;

    const LEGACY_FLAGS: [&str; 6] = [
        "SYSTEM_UI_FLAG_IMMERSIVE_STICKY",
        "SYSTEM_UI_FLAG_FULLSCREEN",
        "SYSTEM_UI_FLAG_HIDE_NAVIGATION",
        "SYSTEM_UI_FLAG_LAYOUT_STABLE",
        "SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN",
        "SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION",
    ];

    /// Runs on the webview's thread, which is the Android UI thread.
    pub fn set_system_bars_hidden(
        env: &mut JNIEnv<'_>,
        activity: &JObject<'_>,
        hidden: bool,
    ) -> Result<(), jni::errors::Error> {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        if window.is_null() {
            return Ok(());
        }

        let sdk_int = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;

        if sdk_int >= 30 {
            // The window must stop fitting the system windows, or the layout
            // keeps the space the hidden bars used to occupy.
            env.call_method(
                &window,
                "setDecorFitsSystemWindows",
                "(Z)V",
                &[JValue::Bool(u8::from(!hidden))],
            )?;

            let controller = env
                .call_method(
                    &window,
                    "getInsetsController",
                    "()Landroid/view/WindowInsetsController;",
                    &[],
                )?
                .l()?;
            if controller.is_null() {
                return Ok(());
            }

            let system_bars = env
                .call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?
                .i()?;

            if hidden {
                // BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE: an edge swipe reveals
                // the bars briefly instead of leaving immersive mode, which is
                // what an unattended display wants — an accidental swipe must
                // not put the bars back for the next hour.
                let behavior = env
                    .get_static_field(
                        "android/view/WindowInsetsController",
                        "BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE",
                        "I",
                    )?
                    .i()?;
                env.call_method(
                    &controller,
                    "setSystemBarsBehavior",
                    "(I)V",
                    &[JValue::Int(behavior)],
                )?;
                env.call_method(&controller, "hide", "(I)V", &[JValue::Int(system_bars)])?;
            } else {
                env.call_method(&controller, "show", "(I)V", &[JValue::Int(system_bars)])?;
            }

            return Ok(());
        }

        let decor_view = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        if decor_view.is_null() {
            return Ok(());
        }

        let mut flags = 0;
        if hidden {
            for name in LEGACY_FLAGS {
                flags |= env.get_static_field("android/view/View", name, "I")?.i()?;
            }
        }
        env.call_method(
            &decor_view,
            "setSystemUiVisibility",
            "(I)V",
            &[JValue::Int(flags)],
        )?;

        Ok(())
    }
}

/// Hides or restores the Android status and navigation bars.
///
/// Called by Mobile whenever kiosk presentation is entered or left, so the
/// bars are gone for both dashboard browsing and the screensaver and come back
/// the moment the authenticated exit completes. It changes only window
/// decoration: back gestures continue to be dispatched to the activity and
/// reach the existing `onBackButtonPress` handler unchanged.
///
/// Every failure is reported, never fatal. Losing immersive mode is cosmetic —
/// a display that keeps its system bars is still a working kiosk — and this
/// runs on the UI thread, where panicking would take the app down with it.
#[tauri::command]
pub fn set_immersive_mode(webview: tauri::Webview, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        webview
            .with_webview(move |platform| {
                platform.jni_handle().exec(move |env, activity, _webview| {
                    if let Err(error) = android::set_system_bars_hidden(env, activity, enabled) {
                        eprintln!("could not change Android system bar visibility: {error}");
                        // A pending Java exception must be cleared, or the next
                        // JNI call on this thread aborts the process.
                        let _ = env.exception_clear();
                    }
                });
            })
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        // Every other platform the mobile shell builds for draws its own
        // window decoration; there is nothing to hide.
        let _ = (&webview, enabled);
    }
    Ok(())
}
