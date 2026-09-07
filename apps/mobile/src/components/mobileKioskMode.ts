import * as React from "react";

export type MobileKioskModeContextValue = {
  isLoading: boolean;
  isTransitioning: boolean;
  enabled: boolean;
  accountId: string | null;
  screensaverEnabled: boolean;
  screensaverIdleSeconds: number;
  enable: (accountId: string) => Promise<void>;
  disable: () => Promise<void>;
  setScreensaverEnabled: (enabled: boolean) => Promise<void>;
  setScreensaverIdleSeconds: (seconds: number) => Promise<void>;
  exitWith: (activateDifferentAccount: () => Promise<void>) => Promise<void>;
};

export const MobileKioskModeContext =
  React.createContext<MobileKioskModeContextValue | null>(null);

export function useMobileKioskMode(): MobileKioskModeContextValue {
  const value = React.useContext(MobileKioskModeContext);
  if (value === null) {
    throw new Error("useMobileKioskMode must be used within MobileKioskModeProvider");
  }
  return value;
}
