import * as React from "react";

import {
  DEFAULT_SCREENSAVER_IDLE_SECONDS,
  disableMobileKioskMode,
  enableMobileKioskMode,
  getMobileKioskSettings,
  setMobileScreensaverEnabled,
  setMobileScreensaverIdleSeconds,
} from "@/adapters/mobileKioskSettings";
import { MobileKioskModeContext } from "@/components/mobileKioskMode";

export function MobileKioskModeProvider({ children }: { children: React.ReactNode }) {
  const [isLoading, setIsLoading] = React.useState(true);
  const [isTransitioning, setIsTransitioning] = React.useState(false);
  const [enabled, setEnabled] = React.useState(false);
  const [accountId, setAccountId] = React.useState<string | null>(null);
  const [screensaverEnabled, setScreensaverEnabledState] = React.useState(true);
  const [screensaverIdleSeconds, setScreensaverIdleSecondsState] = React.useState(
    DEFAULT_SCREENSAVER_IDLE_SECONDS,
  );

  React.useEffect(() => {
    let cancelled = false;
    void getMobileKioskSettings()
      .then((settings) => {
        if (cancelled) return;
        setEnabled(settings.enabled);
        setAccountId(settings.accountId);
        setScreensaverEnabledState(settings.screensaverEnabled);
        setScreensaverIdleSecondsState(settings.screensaverIdleSeconds);
      })
      .catch(() => {
        if (cancelled) return;
        setEnabled(false);
        setAccountId(null);
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const enable = React.useCallback(async (nextAccountId: string) => {
    await enableMobileKioskMode(nextAccountId);
    setAccountId(nextAccountId);
    setEnabled(true);
  }, []);

  const disable = React.useCallback(async () => {
    await disableMobileKioskMode();
    setEnabled(false);
    setAccountId(null);
  }, []);

  const setScreensaverEnabled = React.useCallback(async (next: boolean) => {
    await setMobileScreensaverEnabled(next);
    setScreensaverEnabledState(next);
  }, []);

  const setScreensaverIdleSeconds = React.useCallback(async (next: number) => {
    const normalized = Math.max(1, Math.round(next));
    await setMobileScreensaverIdleSeconds(normalized);
    setScreensaverIdleSecondsState(normalized);
  }, []);

  const exitWith = React.useCallback(
    async (activateDifferentAccount: () => Promise<void>) => {
      setIsTransitioning(true);
      try {
        // Persist the verified replacement session first. Until the Store flag
        // is cleared the top-level gate remains closed; a crash or Store error
        // here therefore lands on same-account recovery, never normal chrome.
        await activateDifferentAccount();
        await disableMobileKioskMode();
        setEnabled(false);
        setAccountId(null);
      } finally {
        setIsTransitioning(false);
      }
    },
    [],
  );

  const value = React.useMemo(
    () => ({
      isLoading,
      isTransitioning,
      enabled,
      accountId,
      screensaverEnabled,
      screensaverIdleSeconds,
      enable,
      disable,
      setScreensaverEnabled,
      setScreensaverIdleSeconds,
      exitWith,
    }),
    [
      accountId,
      disable,
      enable,
      enabled,
      exitWith,
      isLoading,
      isTransitioning,
      screensaverEnabled,
      screensaverIdleSeconds,
      setScreensaverEnabled,
      setScreensaverIdleSeconds,
    ],
  );

  return (
    <MobileKioskModeContext.Provider value={value}>
      {children}
    </MobileKioskModeContext.Provider>
  );
}
