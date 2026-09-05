import * as React from "react";

import {
  disableMobileKioskMode,
  enableMobileKioskMode,
  getMobileKioskSettings,
} from "@/adapters/mobileKioskSettings";
import { MobileKioskModeContext } from "@/components/mobileKioskMode";

export function MobileKioskModeProvider({ children }: { children: React.ReactNode }) {
  const [isLoading, setIsLoading] = React.useState(true);
  const [isTransitioning, setIsTransitioning] = React.useState(false);
  const [enabled, setEnabled] = React.useState(false);
  const [accountId, setAccountId] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    void getMobileKioskSettings()
      .then((settings) => {
        if (cancelled) return;
        setEnabled(settings.enabled);
        setAccountId(settings.accountId);
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
    () => ({ isLoading, isTransitioning, enabled, accountId, enable, disable, exitWith }),
    [accountId, disable, enable, enabled, exitWith, isLoading, isTransitioning],
  );

  return (
    <MobileKioskModeContext.Provider value={value}>
      {children}
    </MobileKioskModeContext.Provider>
  );
}
