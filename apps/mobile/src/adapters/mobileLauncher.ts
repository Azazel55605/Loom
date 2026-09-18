import { invoke, isTauri } from "@tauri-apps/api/core";

/** What Android was able to show when asked to choose a home app. */
export type HomeSettingsOutcome =
  | "rolePrompt"
  | "homeSettings"
  | "alreadyHomeApp"
  | "unavailable";

/**
 * Whether this launch was started as the device's home screen.
 *
 * Per-launch, deliberately: the same installation is the home screen when the
 * Home button starts it and an ordinary app when its icon does, and only the
 * Intent that started this instance knows which happened. Nothing is stored.
 */
export async function isActingAsLauncher(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    return await invoke<boolean>("is_acting_as_launcher");
  } catch {
    // Older shells without the command, and non-Android hosts.
    return false;
  }
}

/** Opens Android's own home-app selection UI. Loom cannot select itself. */
export async function openHomeAppSettings(): Promise<HomeSettingsOutcome> {
  if (!isTauri()) return "unavailable";
  try {
    return await invoke<HomeSettingsOutcome>("open_home_app_settings");
  } catch {
    return "unavailable";
  }
}
