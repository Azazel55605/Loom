import { mobileSettingsStore } from "@/adapters/mobileSettings";

const KIOSK_MODE_ENABLED_KEY = "kioskModeEnabled";
const KIOSK_ACCOUNT_ID_KEY = "kioskAccountId";
const SCREENSAVER_ENABLED_KEY = "screensaverEnabled";
const SCREENSAVER_IDLE_SECONDS_KEY = "screensaverIdleSeconds";

export const DEFAULT_SCREENSAVER_IDLE_SECONDS = 120;

export type MobileKioskSettings = {
  enabled: boolean;
  accountId: string | null;
  screensaverEnabled: boolean;
  screensaverIdleSeconds: number;
};

/** Non-sensitive, device-local kiosk presentation state. */
export async function getMobileKioskSettings(): Promise<MobileKioskSettings> {
  const store = await mobileSettingsStore();
  const [enabled, accountId, screensaverEnabled, screensaverIdleSeconds] = await Promise.all([
    store.get<unknown>(KIOSK_MODE_ENABLED_KEY),
    store.get<unknown>(KIOSK_ACCOUNT_ID_KEY),
    store.get<unknown>(SCREENSAVER_ENABLED_KEY),
    store.get<unknown>(SCREENSAVER_IDLE_SECONDS_KEY),
  ]);
  return {
    enabled: enabled === true,
    accountId: typeof accountId === "string" && accountId !== "" ? accountId : null,
    screensaverEnabled: screensaverEnabled !== false,
    screensaverIdleSeconds:
      typeof screensaverIdleSeconds === "number" &&
      Number.isFinite(screensaverIdleSeconds) &&
      screensaverIdleSeconds > 0
        ? Math.round(screensaverIdleSeconds)
        : DEFAULT_SCREENSAVER_IDLE_SECONDS,
  };
}

export async function enableMobileKioskMode(accountId: string): Promise<void> {
  const store = await mobileSettingsStore();
  const existingScreensaverEnabled = await store.get<unknown>(SCREENSAVER_ENABLED_KEY);
  await Promise.all([
    store.set(KIOSK_MODE_ENABLED_KEY, true),
    store.set(KIOSK_ACCOUNT_ID_KEY, accountId),
    ...(typeof existingScreensaverEnabled === "boolean"
      ? []
      : [store.set(SCREENSAVER_ENABLED_KEY, true)]),
  ]);
  await store.save();
}

export async function setMobileScreensaverEnabled(enabled: boolean): Promise<void> {
  const store = await mobileSettingsStore();
  await store.set(SCREENSAVER_ENABLED_KEY, enabled);
  await store.save();
}

export async function setMobileScreensaverIdleSeconds(seconds: number): Promise<void> {
  const store = await mobileSettingsStore();
  await store.set(SCREENSAVER_IDLE_SECONDS_KEY, Math.max(1, Math.round(seconds)));
  await store.save();
}

export async function disableMobileKioskMode(): Promise<void> {
  const store = await mobileSettingsStore();
  await Promise.all([
    store.set(KIOSK_MODE_ENABLED_KEY, false),
    store.delete(KIOSK_ACCOUNT_ID_KEY),
  ]);
  await store.save();
}
