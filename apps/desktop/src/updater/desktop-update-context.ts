import * as React from "react";

export const DESKTOP_RELEASES_URL =
  "https://github.com/Azazel55605/Loom/releases";

export type DesktopPlatform = "windows" | "macos" | "linux" | "unknown";

export interface DesktopPlatformInfo {
  os: DesktopPlatform;
  isFlatpak: boolean;
}

export interface DesktopUpdateSummary {
  version: string;
  notes?: string;
  publishedAt?: string;
}

export interface DesktopUpdateContextValue {
  platform: DesktopPlatformInfo | null;
  update: DesktopUpdateSummary | null;
  checking: boolean;
  downloading: boolean;
  downloaded: boolean;
  downloadedBytes: number;
  downloadSize: number | null;
  error: string | null;
  checkNow: () => Promise<void>;
  download: () => Promise<void>;
  install: () => Promise<void>;
}

export const DesktopUpdateContext =
  React.createContext<DesktopUpdateContextValue | null>(null);

export function useDesktopUpdates(): DesktopUpdateContextValue {
  const value = React.useContext(DesktopUpdateContext);
  if (value === null) {
    throw new Error("useDesktopUpdates must be used inside DesktopUpdateProvider");
  }
  return value;
}

function semverParts(version: string): number[] | null {
  const normalized = version.trim().replace(/^v/, "").split("-", 1)[0];
  const parts = normalized.split(".");
  if (parts.length !== 3 || parts.some((part) => !/^\d+$/.test(part))) return null;
  return parts.map(Number);
}

/** Compare the release versions Loom writes through versions.json. */
export function isNewerDesktopVersion(candidate: string, current: string): boolean {
  const candidateParts = semverParts(candidate);
  const currentParts = semverParts(current);
  if (candidateParts === null || currentParts === null) return false;
  for (let index = 0; index < 3; index += 1) {
    if (candidateParts[index] !== currentParts[index]) {
      return candidateParts[index] > currentParts[index];
    }
  }
  return false;
}
