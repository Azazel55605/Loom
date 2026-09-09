import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Download, ExternalLink, Loader2, RefreshCw } from "lucide-react";

import {
  DESKTOP_RELEASES_URL,
  useDesktopUpdates,
} from "@/updater/desktop-update-context";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@loom/ui-kit/components/ui/alert-dialog";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { Progress } from "@loom/ui-kit/components/ui/progress";

function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB"];
  let scaled = value / 1024;
  let unit = units[0];
  for (let index = 1; index < units.length && scaled >= 1024; index += 1) {
    scaled /= 1024;
    unit = units[index];
  }
  return `${scaled.toFixed(scaled >= 10 ? 0 : 1)} ${unit}`;
}

function platformMessage(
  os: "windows" | "macos" | "linux" | "unknown",
  isFlatpak: boolean,
): string {
  if (isFlatpak) {
    return "This app is managed through Flatpak — run `flatpak update` to get the latest version.";
  }
  if (os === "linux") {
    return "This app is managed through your system's package manager (apt, dnf, or your AUR helper) — check there for updates.";
  }
  if (os === "macos") {
    return "Updates are downloaded from the published GitHub release and installed manually.";
  }
  if (os === "windows") {
    return "Windows updates can be downloaded, signature-verified, and installed here.";
  }
  return "This platform can check published releases but does not support in-app installation.";
}

export function DesktopUpdatesPanel() {
  const updater = useDesktopUpdates();
  const [confirmInstall, setConfirmInstall] = React.useState(false);
  const downloadPercent =
    updater.downloadSize === null || updater.downloadSize === 0
      ? null
      : Math.min(100, (updater.downloadedBytes / updater.downloadSize) * 100);
  const platform = updater.platform;
  const isWindows = platform?.os === "windows";

  React.useEffect(() => {
    if (updater.downloaded) setConfirmInstall(true);
  }, [updater.downloaded]);

  return (
    <>
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className="text-lg">Desktop updates</CardTitle>
            <Badge variant="outline">v{__APP_VERSION__}</Badge>
          </div>
          <CardDescription>
            Loom checks published desktop releases. Downloads and installation always
            require your action.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {platform !== null ? (
            <p className="text-sm text-muted-foreground">
              {platformMessage(platform.os, platform.isFlatpak)}
            </p>
          ) : null}

          {updater.error !== null ? (
            <Alert variant="destructive">
              <AlertTitle>Update check failed</AlertTitle>
              <AlertDescription>{updater.error}</AlertDescription>
            </Alert>
          ) : null}

          {updater.update === null && !updater.checking && updater.error === null ? (
            <p className="text-sm">You are running the latest published version.</p>
          ) : null}

          {updater.update !== null ? (
            <Alert>
              <AlertTitle>Update available: v{updater.update.version}</AlertTitle>
              <AlertDescription className="flex flex-col gap-2">
                {updater.update.notes ? (
                  <span className="whitespace-pre-wrap">{updater.update.notes}</span>
                ) : null}
                {updater.update.publishedAt ? (
                  <span>Published {new Date(updater.update.publishedAt).toLocaleString()}</span>
                ) : null}
              </AlertDescription>
            </Alert>
          ) : null}

          {updater.downloading ? (
            <div className="flex flex-col gap-2" aria-live="polite">
              {downloadPercent === null ? null : <Progress value={downloadPercent} />}
              <p className="text-sm text-muted-foreground">
                Downloaded {bytes(updater.downloadedBytes)}
                {updater.downloadSize === null ? "" : ` of ${bytes(updater.downloadSize)}`}
              </p>
            </div>
          ) : null}
        </CardContent>
        <CardFooter className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={updater.checking || updater.downloading}
            onClick={() => void updater.checkNow()}
          >
            {updater.checking ? (
              <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw data-icon="inline-start" aria-hidden="true" />
            )}
            Check for Updates
          </Button>

          {isWindows && updater.update !== null ? (
            <Button
              type="button"
              disabled={updater.downloading}
              onClick={() => {
                if (updater.downloaded) {
                  setConfirmInstall(true);
                } else {
                  void updater.download();
                }
              }}
            >
              {updater.downloading ? (
                <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
              ) : (
                <Download data-icon="inline-start" aria-hidden="true" />
              )}
              {updater.downloaded ? "Install and restart" : "Download and Install"}
            </Button>
          ) : null}

          {!isWindows && !platform?.isFlatpak && updater.update !== null ? (
            <Button
              type="button"
              onClick={() => void openUrl(DESKTOP_RELEASES_URL)}
            >
              <ExternalLink data-icon="inline-start" aria-hidden="true" />
              Open GitHub release
            </Button>
          ) : null}
        </CardFooter>
      </Card>

      <AlertDialog open={confirmInstall} onOpenChange={setConfirmInstall}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Install update and restart Loom?</AlertDialogTitle>
            <AlertDialogDescription>
              The update is downloaded and its signature will be verified before
              installation. Loom will close, install v{updater.update?.version}, and
              restart.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Later</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => void updater.install()}
            >
              Install and restart
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
