import * as React from "react";
import { AlertCircle } from "lucide-react";

import { desktopBaseUrlProvider } from "@/adapters/desktopBaseUrlProvider";
import { normalizeServerUrl } from "@/lib/server-url";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";

type Health = { status: string; core_version: string };

export function ConnectToServer({
  initialUrl = "",
  embedded = false,
  onConnected,
}: {
  initialUrl?: string;
  embedded?: boolean;
  onConnected: (baseUrl: string) => void | Promise<void>;
}) {
  const [draft, setDraft] = React.useState(initialUrl);
  const [error, setError] = React.useState<string | null>(null);
  const [isConnecting, setIsConnecting] = React.useState(false);

  React.useEffect(() => setDraft(initialUrl), [initialUrl]);

  const form = (
    <form
      className="flex flex-col gap-4"
      onSubmit={async (event) => {
        event.preventDefault();
        setError(null);

        let baseUrl: string;
        try {
          baseUrl = normalizeServerUrl(draft);
        } catch (validationError) {
          setError(
            validationError instanceof Error
              ? validationError.message
              : "Enter a valid server URL.",
          );
          return;
        }

        setIsConnecting(true);
        const controller = new AbortController();
        const timeout = window.setTimeout(() => controller.abort(), 8_000);
        try {
          const response = await fetch(`${baseUrl}/health`, {
            signal: controller.signal,
          });
          if (!response.ok) {
            throw new Error(`The server returned ${response.status}.`);
          }
          const health = (await response.json()) as Partial<Health>;
          if (health.status !== "ok" || typeof health.core_version !== "string") {
            throw new Error("The address did not return a Loom health response.");
          }
          await desktopBaseUrlProvider.setBaseUrl(baseUrl);
          await onConnected(baseUrl);
        } catch (connectionError) {
          setError(
            connectionError instanceof DOMException && connectionError.name === "AbortError"
              ? "The server did not respond in time."
              : connectionError instanceof Error
                ? connectionError.message
                : "The server could not be reached.",
          );
        } finally {
          window.clearTimeout(timeout);
          setIsConnecting(false);
        }
      }}
    >
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={embedded ? "settings-server-url" : "server-url"}>
          Server URL
        </Label>
        <Input
          id={embedded ? "settings-server-url" : "server-url"}
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          placeholder="https://your-server:8080"
          value={draft}
          aria-invalid={error !== null}
          onChange={(event) => setDraft(event.target.value)}
        />
      </div>

      {error !== null ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not connect</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}

      <Button type="submit" disabled={isConnecting || draft.trim() === ""}>
        {isConnecting ? "Connecting…" : embedded ? "Change server" : "Connect"}
      </Button>
    </form>
  );

  if (embedded) return form;

  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <Card className="surface-elevated w-full max-w-md">
        <CardHeader>
          <CardTitle>Connect to Loom</CardTitle>
          <CardDescription>
            Enter the address of the Loom server this desktop app should manage.
          </CardDescription>
        </CardHeader>
        <CardContent>{form}</CardContent>
      </Card>
    </main>
  );
}
