import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useMobileKioskMode } from "@/components/mobileKioskMode";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import { useApiClient } from "@loom/ui-kit/lib/api-context";

export function MobileKioskSettingsCard() {
  const api = useApiClient();
  const kiosk = useMobileKioskMode();
  const [failure, setFailure] = React.useState<string | null>(null);
  const [idleSeconds, setIdleSeconds] = React.useState("120");
  const account = useQuery({
    queryKey: ["account"],
    queryFn: ({ signal }) => api.getAccount(signal),
  });
  const eligible = account.data?.isKiosk === true;
  const disabled = account.isPending || account.isError || (!eligible && !kiosk.enabled);

  React.useEffect(() => {
    setIdleSeconds(String(kiosk.screensaverIdleSeconds));
  }, [kiosk.screensaverIdleSeconds]);

  function saveIdleSeconds() {
    const parsed = Number(idleSeconds);
    if (!Number.isFinite(parsed) || parsed < 1) {
      setFailure("Idle timeout must be at least one second.");
      setIdleSeconds(String(kiosk.screensaverIdleSeconds));
      return;
    }
    setFailure(null);
    void kiosk.setScreensaverIdleSeconds(parsed).catch(() => {
      setFailure("Could not save the screensaver timeout on this device.");
    });
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Kiosk presentation</CardTitle>
        <CardDescription>
          Show assigned dashboards without navigation or administrative chrome on this device.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex min-h-11 items-center justify-between gap-4 rounded-md border p-3">
          <div className="flex flex-col gap-1">
            <Label htmlFor="mobile-kiosk-mode">Enable Kiosk Mode</Label>
            <p className="text-xs text-muted-foreground">
              {eligible
                ? "Exit requires signing in with a different, non-kiosk account."
                : "Log in as a kiosk-designated account to enable this."}
            </p>
          </div>
          <Switch
            id="mobile-kiosk-mode"
            checked={kiosk.enabled}
            disabled={disabled}
            onCheckedChange={(checked) => {
              setFailure(null);
              if (checked && account.data !== undefined) {
                void kiosk.enable(account.data.id).catch(() => {
                  setFailure("Could not save the kiosk setting on this device.");
                });
              } else if (!checked) {
                void kiosk.disable().catch(() => {
                  setFailure("Could not save the kiosk setting on this device.");
                });
              }
            }}
          />
        </div>
        {eligible ? (
          <div className="flex flex-col gap-3 rounded-md border p-3">
            <div className="flex min-h-11 items-center justify-between gap-4">
              <div className="flex flex-col gap-1">
                <Label htmlFor="mobile-kiosk-screensaver">Idle screensaver</Label>
                <p className="text-xs text-muted-foreground">
                  Show the clock and administrator-selected live stats while idle.
                </p>
              </div>
              <Switch
                id="mobile-kiosk-screensaver"
                checked={kiosk.screensaverEnabled}
                onCheckedChange={(checked) => {
                  setFailure(null);
                  void kiosk.setScreensaverEnabled(checked).catch(() => {
                    setFailure("Could not save the screensaver setting on this device.");
                  });
                }}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="mobile-kiosk-idle-seconds">Start after (seconds)</Label>
              <Input
                id="mobile-kiosk-idle-seconds"
                type="number"
                min={1}
                inputMode="numeric"
                value={idleSeconds}
                disabled={!kiosk.screensaverEnabled}
                onChange={(event) => setIdleSeconds(event.target.value)}
                onBlur={saveIdleSeconds}
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                }}
              />
              <p className="text-xs text-muted-foreground">
                Stored only on this device. The default is 120 seconds.
              </p>
            </div>
          </div>
        ) : null}
        {failure !== null ? (
          <p className="mt-2 text-sm text-destructive" role="alert">
            {failure}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
