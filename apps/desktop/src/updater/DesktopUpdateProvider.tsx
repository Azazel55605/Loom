import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

import {
  DesktopUpdateContext,
  isNewerDesktopVersion,
  type DesktopPlatformInfo,
  type DesktopUpdateContextValue,
  type DesktopUpdateSummary,
} from "@/updater/desktop-update-context";

const LATEST_MANIFEST_URL =
  "https://github.com/Azazel55605/Loom/releases/latest/download/latest.json";
interface LatestManifest {
  version?: unknown;
  notes?: unknown;
  pub_date?: unknown;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function loadPlatform(): Promise<DesktopPlatformInfo> {
  return invoke<DesktopPlatformInfo>("desktop_platform_info");
}

async function checkPublishedManifest(): Promise<DesktopUpdateSummary | null> {
  const response = await tauriFetch(LATEST_MANIFEST_URL, {
    method: "GET",
    redirect: "follow",
  });
  if (!response.ok) {
    throw new Error(`GitHub returned HTTP ${response.status} while checking updates.`);
  }
  const manifest = (await response.json()) as LatestManifest;
  if (typeof manifest.version !== "string") {
    throw new Error("The published update manifest has no valid version.");
  }
  if (!isNewerDesktopVersion(manifest.version, __APP_VERSION__)) return null;
  return {
    version: manifest.version.replace(/^v/, ""),
    notes: typeof manifest.notes === "string" ? manifest.notes : undefined,
    publishedAt:
      typeof manifest.pub_date === "string" ? manifest.pub_date : undefined,
  };
}

export function DesktopUpdateProvider({ children }: { children: React.ReactNode }) {
  const [platform, setPlatform] = React.useState<DesktopPlatformInfo | null>(null);
  const [update, setUpdate] = React.useState<DesktopUpdateSummary | null>(null);
  const [checking, setChecking] = React.useState(false);
  const [downloading, setDownloading] = React.useState(false);
  const [downloaded, setDownloaded] = React.useState(false);
  const [downloadedBytes, setDownloadedBytes] = React.useState(0);
  const [downloadSize, setDownloadSize] = React.useState<number | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const pendingUpdate = React.useRef<Update | null>(null);
  const checkedOnLaunch = React.useRef(false);

  const runCheck = React.useCallback(async (silent: boolean) => {
    setChecking(true);
    if (!silent) setError(null);
    try {
      const detectedPlatform = platform ?? (await loadPlatform());
      setPlatform(detectedPlatform);
      setDownloaded(false);
      setDownloadedBytes(0);
      setDownloadSize(null);

      if (detectedPlatform.os === "windows") {
        const available = await check({ timeout: 30_000 });
        pendingUpdate.current = available;
        setUpdate(
          available === null
            ? null
            : {
                version: available.version,
                notes: available.body,
                publishedAt: available.date,
              },
        );
      } else {
        pendingUpdate.current = null;
        setUpdate(await checkPublishedManifest());
      }
    } catch (caught) {
      if (!silent) setError(errorMessage(caught));
    } finally {
      setChecking(false);
    }
  }, [platform]);

  React.useEffect(() => {
    if (checkedOnLaunch.current) return;
    checkedOnLaunch.current = true;
    void runCheck(true);
  }, [runCheck]);

  const download = React.useCallback(async () => {
    const available = pendingUpdate.current;
    if (available === null) return;
    setError(null);
    setDownloading(true);
    setDownloadedBytes(0);
    setDownloadSize(null);
    try {
      await available.download((event: DownloadEvent) => {
        if (event.event === "Started") {
          setDownloadSize(event.data.contentLength ?? null);
        } else if (event.event === "Progress") {
          setDownloadedBytes((current) => current + event.data.chunkLength);
        } else {
          setDownloaded(true);
        }
      });
      setDownloaded(true);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setDownloading(false);
    }
  }, []);

  const install = React.useCallback(async () => {
    const available = pendingUpdate.current;
    if (available === null || !downloaded) return;
    setError(null);
    try {
      // Windows exits after successfully launching the installer. Asking for
      // confirmation before this call is therefore the last reliable UI point.
      await available.install({ restartAfterInstall: true });
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }, [downloaded]);

  const value = React.useMemo<DesktopUpdateContextValue>(
    () => ({
      platform,
      update,
      checking,
      downloading,
      downloaded,
      downloadedBytes,
      downloadSize,
      error,
      checkNow: () => runCheck(false),
      download,
      install,
    }),
    [
      checking,
      download,
      downloading,
      downloadSize,
      downloaded,
      downloadedBytes,
      error,
      install,
      platform,
      runCheck,
      update,
    ],
  );

  return (
    <DesktopUpdateContext.Provider value={value}>
      {children}
    </DesktopUpdateContext.Provider>
  );
}
