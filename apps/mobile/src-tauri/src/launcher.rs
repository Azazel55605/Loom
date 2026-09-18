//! Android home-app (launcher) registration and detection.
//!
//! Loom's Activity declares a second intent filter — `MAIN` + `HOME` +
//! `DEFAULT` — which is only an *offer*: it makes Loom appear in Android's home
//! app chooser. Android alone decides which app holds the home role, the user
//! alone selects it, and it is revocable from Settings at any time. Nothing
//! here asks for an elevated privilege, and nothing here can make Loom the
//! launcher on its own.
//!
//! Two things are exposed to the frontend:
//!
//! - [`is_acting_as_launcher`], asking the Activity's own start Intent whether
//!   *this* instance was started as the home screen. Per-launch state, read
//!   fresh from the Intent every time; never persisted.
//! - [`open_home_app_settings`], sending the user to Android's own selection
//!   UI, preferring the role request dialog and falling back to the home
//!   settings screen.
//!
//! As in `immersive`, the JNI environment and Activity come from Tauri's
//! webview handle. These calls need an answer back, so the closure sends its
//! result over a channel the command waits on with a timeout — a command must
//! never block forever on a UI thread that may be busy.

#[cfg(target_os = "android")]
use std::time::Duration;

/// Long enough for a UI thread mid-frame, short enough not to hang a setting.
#[cfg(target_os = "android")]
const JNI_TIMEOUT: Duration = Duration::from_secs(5);

/// What [`open_home_app_settings`] managed to open, so the UI knows whether it
/// still has to explain the manual path.
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HomeSettingsOutcome {
    /// Android's own "make Loom your Home app?" dialog is on screen.
    RolePrompt,
    /// Android's home-app selection screen is on screen.
    HomeSettings,
    /// Loom already holds the home role; nothing to ask.
    AlreadyHomeApp,
    /// Neither could be opened. The caller must show the manual instructions.
    Unavailable,
}

#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};
    use jni::JNIEnv;

    use super::HomeSettingsOutcome;

    const CATEGORY_HOME: &str = "android.intent.category.HOME";
    const ROLE_HOME: &str = "android.app.role.HOME";
    const ACTION_HOME_SETTINGS: &str = "android.settings.HOME_SETTINGS";
    /// Ignored: the answer is observed through the role state, not a result.
    const ROLE_REQUEST_CODE: i32 = 0;

    /// Whether the Intent that started this Activity carried `CATEGORY_HOME`.
    ///
    /// That category is what distinguishes the home intent the system sends
    /// when the Home button or gesture is used from `CATEGORY_LAUNCHER`, which
    /// is what tapping the app icon sends.
    pub fn is_home_launch(
        env: &mut JNIEnv<'_>,
        activity: &JObject<'_>,
    ) -> Result<bool, jni::errors::Error> {
        let intent = env
            .call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?
            .l()?;
        if intent.is_null() {
            return Ok(false);
        }
        let category = env.new_string(CATEGORY_HOME)?;
        let present = env
            .call_method(
                &intent,
                "hasCategory",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&category)],
            )?
            .z()?;
        Ok(present)
    }

    pub fn open_home_app_settings(
        env: &mut JNIEnv<'_>,
        activity: &JObject<'_>,
    ) -> Result<HomeSettingsOutcome, jni::errors::Error> {
        let sdk_int = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;

        // `RoleManager` (API 29+) shows a one-tap system dialog. Preferred over
        // the settings screen because it names the choice being made instead of
        // leaving the user to find the right row.
        if sdk_int >= 29 {
            if let Some(outcome) = request_home_role(env, activity)? {
                return Ok(outcome);
            }
        }

        let action = env.new_string(ACTION_HOME_SETTINGS)?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&action)],
        )?;
        match env.call_method(
            activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        ) {
            Ok(_) => Ok(HomeSettingsOutcome::HomeSettings),
            Err(_) => {
                // Not every build ships that settings screen, and the throw
                // must be cleared before the next JNI call on this thread.
                let _ = env.exception_clear();
                Ok(HomeSettingsOutcome::Unavailable)
            }
        }
    }

    /// `None` when the role path is unusable and the caller should fall back.
    fn request_home_role(
        env: &mut JNIEnv<'_>,
        activity: &JObject<'_>,
    ) -> Result<Option<HomeSettingsOutcome>, jni::errors::Error> {
        let service_name = match env.get_static_field(
            "android/content/Context",
            "ROLE_SERVICE",
            "Ljava/lang/String;",
        ) {
            Ok(value) => value.l()?,
            Err(_) => {
                let _ = env.exception_clear();
                return Ok(None);
            }
        };
        let manager = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&service_name)],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(None);
        }

        let role = env.new_string(ROLE_HOME)?;
        // Roles can come and go with a system update, so availability is asked
        // rather than assumed.
        let available = env
            .call_method(
                &manager,
                "isRoleAvailable",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&role)],
            )?
            .z()?;
        if !available {
            return Ok(None);
        }
        let held = env
            .call_method(
                &manager,
                "isRoleHeld",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&role)],
            )?
            .z()?;
        if held {
            return Ok(Some(HomeSettingsOutcome::AlreadyHomeApp));
        }

        let intent = env
            .call_method(
                &manager,
                "createRequestRoleIntent",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&role)],
            )?
            .l()?;
        match env.call_method(
            activity,
            "startActivityForResult",
            "(Landroid/content/Intent;I)V",
            &[JValue::Object(&intent), JValue::Int(ROLE_REQUEST_CODE)],
        ) {
            Ok(_) => Ok(Some(HomeSettingsOutcome::RolePrompt)),
            Err(_) => {
                let _ = env.exception_clear();
                Ok(None)
            }
        }
    }
}

/// Runs a JNI closure on the webview's thread and waits for its answer.
#[cfg(target_os = "android")]
fn with_activity<T, F>(webview: &tauri::Webview, closure: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut jni::JNIEnv, &jni::objects::JObject) -> T + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    webview
        .with_webview(move |platform| {
            platform.jni_handle().exec(move |env, activity, _webview| {
                let _ = sender.send(closure(env, activity));
            });
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(JNI_TIMEOUT)
        .map_err(|_| "the Android UI thread did not answer in time".to_string())
}

/// Whether this app instance was started as the device's home screen.
///
/// Read from the Activity's start Intent on every call rather than stored: it
/// describes *this* launch, and the same installation is regularly both — the
/// home screen on one start, an ordinary app tapped from a list on the next.
#[tauri::command]
pub fn is_acting_as_launcher(webview: tauri::Webview) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        with_activity(&webview, |env, activity| {
            android::is_home_launch(env, activity).unwrap_or_else(|_| {
                let _ = env.exception_clear();
                false
            })
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        // No home-app concept on the other platforms this shell can build for.
        let _ = &webview;
        Ok(false)
    }
}

/// Opens Android's own home-app selection UI.
#[tauri::command]
pub fn open_home_app_settings(webview: tauri::Webview) -> Result<HomeSettingsOutcome, String> {
    #[cfg(target_os = "android")]
    {
        with_activity(&webview, |env, activity| {
            android::open_home_app_settings(env, activity).unwrap_or_else(|_| {
                let _ = env.exception_clear();
                HomeSettingsOutcome::Unavailable
            })
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = &webview;
        Ok(HomeSettingsOutcome::Unavailable)
    }
}
