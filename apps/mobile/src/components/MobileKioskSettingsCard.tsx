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
import { Switch } from "@loom/ui-kit/components/ui/switch";
import { useApiClient } from "@loom/ui-kit/lib/api-context";

export function MobileKioskSettingsCard() {
  const api = useApiClient();
  const kiosk = useMobileKioskMode();
  const [failure, setFailure] = React.useState<string | null>(null);
  const account = useQuery({
    queryKey: ["account"],
    queryFn: ({ signal }) => api.getAccount(signal),
  });
  const eligible = account.data?.isKiosk === true;
  const disabled = account.isPending || account.isError || (!eligible && !kiosk.enabled);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Kiosk presentation</CardTitle>
        <CardDescription>
          Show assigned dashboards without navigation or administrative chrome on this device.
        </CardDescription>
      </CardHeader>
      <CardContent>
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
        {failure !== null ? (
          <p className="mt-2 text-sm text-destructive" role="alert">
            {failure}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
