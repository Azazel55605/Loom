import { mobileSettingsStore } from "@/adapters/mobileSettings";
import { mobileServerProfileManager } from "@/adapters/mobileServerProfileManager";
import type { ServerConnection } from "@loom/ui-kit/components/ConnectToServer";
import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";
import { ProfileBaseUrlProvider, serverProfileLabel } from "@loom/ui-kit/lib/server-profile";

const SERVER_URL_KEY = "serverUrl";
const ALLOW_INVALID_CERTIFICATES_KEY = "allowInvalidCertificates";
const PROFILE_TLS_KEY = "serverProfileAllowInvalidCertificates";
const PROFILE_TLS_MIGRATED_KEY = "serverProfileTlsMigrated";
const profileBaseUrlProvider = new ProfileBaseUrlProvider(mobileServerProfileManager);

function readTlsSettings(value: unknown): Record<string, boolean> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter((entry): entry is [string, boolean] => typeof entry[1] === "boolean"),
  );
}

async function getTlsSettings(): Promise<Record<string, boolean>> {
  const store = await mobileSettingsStore();
  const settings = readTlsSettings(await store.get<unknown>(PROFILE_TLS_KEY));
  if ((await store.get<unknown>(PROFILE_TLS_MIGRATED_KEY)) === true) return settings;

  const [activeProfileId, legacyPolicy] = await Promise.all([
    mobileServerProfileManager.getActiveProfileId(),
    store.get<unknown>(ALLOW_INVALID_CERTIFICATES_KEY),
  ]);
  if (activeProfileId !== null && legacyPolicy === true) settings[activeProfileId] = true;
  await store.set(PROFILE_TLS_KEY, settings);
  await store.set(PROFILE_TLS_MIGRATED_KEY, true);
  await store.save();
  return settings;
}

/** Non-sensitive runtime server configuration persisted by Tauri Store. */
class MobileBaseUrlProvider implements BaseUrlProvider {
  async getBaseUrl(): Promise<string> {
    return profileBaseUrlProvider.getBaseUrl();
  }

  async getConnection(): Promise<ServerConnection> {
    const [profiles, activeProfileId, tlsSettings] = await Promise.all([
      mobileServerProfileManager.listProfiles(),
      mobileServerProfileManager.getActiveProfileId(),
      getTlsSettings(),
    ]);
    const active = profiles.find(({ id }) => id === activeProfileId);
    return {
      baseUrl: active?.baseUrl ?? "",
      allowInvalidCertificates: active ? tlsSettings[active.id] === true : false,
    };
  }

  /** Retained for the existing connection bootstrap and current-profile editor.
   * Profile switching itself goes through `mobileServerProfileManager`. */
  async setConnection(connection: ServerConnection): Promise<void> {
    const store = await mobileSettingsStore();
    const activeProfileId = await mobileServerProfileManager.getActiveProfileId();
    if (!connection.baseUrl) {
      await mobileServerProfileManager.clearActiveProfileId();
      await Promise.all([
        store.set(SERVER_URL_KEY, ""),
        store.set(ALLOW_INVALID_CERTIFICATES_KEY, false),
      ]);
      await store.save();
      return;
    }

    const profileId = activeProfileId ?? (await mobileServerProfileManager.addProfile(
      serverProfileLabel(connection.baseUrl),
      connection.baseUrl,
    )).id;
    if (activeProfileId !== null) {
      await mobileServerProfileManager.updateProfile(profileId, { baseUrl: connection.baseUrl });
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

export const mobileBaseUrlProvider = new MobileBaseUrlProvider();
