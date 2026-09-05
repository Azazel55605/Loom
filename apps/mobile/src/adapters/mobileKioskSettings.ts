import { mobileSettingsStore } from "@/adapters/mobileSettings";

const KIOSK_MODE_ENABLED_KEY = "kioskModeEnabled";
const KIOSK_ACCOUNT_ID_KEY = "kioskAccountId";

export type MobileKioskSettings = {
  enabled: boolean;
  accountId: string | null;
};

/** Non-sensitive, device-local kiosk presentation state. */
export async function getMobileKioskSettings(): Promise<MobileKioskSettings> {
  const store = await mobileSettingsStore();
  const [enabled, accountId] = await Promise.all([
    store.get<unknown>(KIOSK_MODE_ENABLED_KEY),
    store.get<unknown>(KIOSK_ACCOUNT_ID_KEY),
  ]);
  return {
    enabled: enabled === true,
    accountId: typeof accountId === "string" && accountId !== "" ? accountId : null,
  };
}

export async function enableMobileKioskMode(accountId: string): Promise<void> {
  const store = await mobileSettingsStore();
  await Promise.all([
    store.set(KIOSK_MODE_ENABLED_KEY, true),
    store.set(KIOSK_ACCOUNT_ID_KEY, accountId),
  ]);
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
