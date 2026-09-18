import * as React from "react";

import {
  isActingAsLauncher,
  openHomeAppSettings,
  type HomeSettingsOutcome,
} from "@/adapters/mobileLauncher";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";

/** Where to find the setting by hand when neither system screen can be opened. */
const MANUAL_PATH = "Settings › Apps › Default apps › Home app";

export function MobileHomeAppSettingsCard() {
  const [outcome, setOutcome] = React.useState<HomeSettingsOutcome | null>(null);
  const [pending, setPending] = React.useState(false);
  const [actingAsLauncher, setActingAsLauncher] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    void isActingAsLauncher().then((value) => {
      if (!cancelled) setActingAsLauncher(value);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Home app</CardTitle>
        <CardDescription>
          Controls what opens when the device&apos;s Home button or gesture is used. This is
          separate from Kiosk Mode, which controls how Loom presents dashboards once it is open.
          Either can be used without the other.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex min-h-11 flex-wrap items-center justify-between gap-3 rounded-md border p-3">
          <p className="text-xs text-muted-foreground">
            {actingAsLauncher
              ? "This session was started as the device's home screen."
              : "Android asks you to confirm the choice, and it stays changeable there."}
          </p>
          <Button
            type="button"
            disabled={pending}
            onClick={() => {
              setPending(true);
              setOutcome(null);
              void openHomeAppSettings()
                .then(setOutcome)
                .finally(() => setPending(false));
            }}
          >
            Set as Home App
          </Button>
        </div>
        {outcome === "homeSettings" ? (
          <p className="text-xs text-muted-foreground" role="status">
            Choose Loom on the home-app screen that just opened.
          </p>
        ) : null}
        {outcome === "rolePrompt" ? (
          <p className="text-xs text-muted-foreground" role="status">
            Confirm Loom in the system dialog that just opened.
          </p>
        ) : null}
        {outcome === "alreadyHomeApp" ? (
          <p className="text-xs text-muted-foreground" role="status">
            Loom is already this device&apos;s home app. Change it back from {MANUAL_PATH}.
          </p>
        ) : null}
        {outcome === "unavailable" ? (
          <p className="text-xs text-muted-foreground" role="status">
            This device does not open that screen directly. Select Loom by hand under{" "}
            {MANUAL_PATH}.
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
