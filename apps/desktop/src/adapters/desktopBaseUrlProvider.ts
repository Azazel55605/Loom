import { desktopServerProfileManager } from "@/adapters/desktopServerProfileManager";
import { desktopSettingsStore } from "@/adapters/desktopSettings";
import type { ServerConnection } from "@loom/ui-kit/components/ConnectToServer";
import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";
import { ProfileBaseUrlProvider, serverProfileLabel } from "@loom/ui-kit/lib/server-profile";

const SERVER_URL_KEY = "serverUrl";
const ALLOW_INVALID_CERTIFICATES_KEY = "allowInvalidCertificates";
const PROFILE_TLS_KEY = "serverProfileAllowInvalidCertificates";
const PROFILE_TLS_MIGRATED_KEY = "serverProfileTlsMigrated";
const profileBaseUrlProvider = new ProfileBaseUrlProvider(desktopServerProfileManager);

function readTlsSettings(value: unknown): Record<string, boolean> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter((entry): entry is [string, boolean] => typeof entry[1] === "boolean"),
  );
}

async function getTlsSettings(): Promise<Record<string, boolean>> {
  const store = await desktopSettingsStore();
  const settings = readTlsSettings(await store.get<unknown>(PROFILE_TLS_KEY));
  if ((await store.get<unknown>(PROFILE_TLS_MIGRATED_KEY)) === true) return settings;

  const [activeProfileId, legacyPolicy] = await Promise.all([
    desktopServerProfileManager.getActiveProfileId(),
    store.get<unknown>(ALLOW_INVALID_CERTIFICATES_KEY),
  ]);
  if (activeProfileId !== null && legacyPolicy === true) settings[activeProfileId] = true;
  await store.set(PROFILE_TLS_KEY, settings);
  await store.set(PROFILE_TLS_MIGRATED_KEY, true);
  await store.save();
  return settings;
}

/** Non-sensitive runtime server configuration persisted by Tauri Store. */
class DesktopBaseUrlProvider implements BaseUrlProvider {
  async getBaseUrl(): Promise<string> {
    return profileBaseUrlProvider.getBaseUrl();
  }

  async getConnection(): Promise<ServerConnection> {
    const [profiles, activeProfileId, tlsSettings] = await Promise.all([
      desktopServerProfileManager.listProfiles(),
      desktopServerProfileManager.getActiveProfileId(),
      getTlsSettings(),
    ]);
    const active = profiles.find(({ id }) => id === activeProfileId);
    return {
      baseUrl: active?.baseUrl ?? "",
      allowInvalidCertificates: active ? tlsSettings[active.id] === true : false,
    };
  }

  /** Retained for the existing connection bootstrap and current-profile editor.
   * Profile switching itself goes through `desktopServerProfileManager`. */
  async setConnection(connection: ServerConnection): Promise<void> {
    const store = await desktopSettingsStore();
    const activeProfileId = await desktopServerProfileManager.getActiveProfileId();
    if (!connection.baseUrl) {
      await desktopServerProfileManager.clearActiveProfileId();
      await Promise.all([
        store.set(SERVER_URL_KEY, ""),
        store.set(ALLOW_INVALID_CERTIFICATES_KEY, false),
      ]);
      await store.save();
      return;
    }

    const profileId = activeProfileId ?? (await desktopServerProfileManager.addProfile(
      serverProfileLabel(connection.baseUrl),
      connection.baseUrl,
    )).id;
    if (activeProfileId !== null) {
      await desktopServerProfileManager.updateProfile(profileId, { baseUrl: connection.baseUrl });
    }
    const tlsSettings = await getTlsSettings();
    tlsSettings[profileId] = connection.allowInvalidCertificates;
    await Promise.all([
      store.set(PROFILE_TLS_KEY, tlsSettings),
      store.set(SERVER_URL_KEY, connection.baseUrl),
      store.set(
        ALLOW_INVALID_CERTIFICATES_KEY,
        connection.allowInvalidCertificates,
      ),
    ]);
    await store.save();
  }
}

export const desktopBaseUrlProvider = new DesktopBaseUrlProvider();
