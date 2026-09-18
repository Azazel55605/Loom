import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Hides or restores the Android status and navigation bars.
 *
 * Kiosk presentation owns this: the bars stay hidden for dashboard browsing and
 * for the screensaver alike, and come back as soon as kiosk mode is left. See
 * `src-tauri/src/immersive.rs` for why this is a command of Loom's own rather
 * than a Tauri API or plugin.
 *
 * Failure is swallowed deliberately. Immersive mode is presentation polish; a
 * display that keeps its system bars is still a working kiosk, and there is
 * nobody standing in front of an unattended tablet to act on an error.
 */
export async function setMobileImmersiveMode(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_immersive_mode", { enabled });
  } catch {
    // Older shells without the command, and non-Android hosts.
  }
}
