import * as React from "react";
import { AlertCircle, TriangleAlert } from "lucide-react";

import {
  desktopBaseUrlProvider,
  type DesktopServerConnection,
} from "@/adapters/desktopBaseUrlProvider";
import { createDesktopHttpTransport } from "@/adapters/desktopHttpTransport";
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
import { Switch } from "@loom/ui-kit/components/ui/switch";

type Health = { status: string; core_version: string };

function connectionErrorMessage(
  error: unknown,
  baseUrl: string,
  allowInvalidCertificates: boolean,
): string {
  if (error instanceof DOMException && error.name === "AbortError") {
    return "The server did not respond in time.";
  }

  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "The server could not be reached.";
  if (
    baseUrl.startsWith("https://") &&
    !allowInvalidCertificates &&
    /load failed|certificate|tls|ssl/i.test(message)
  ) {
    return "The TLS connection failed. If this server uses a self-signed certificate, enable the certificate exception below and try again.";
  }
  return message;
}

export function ConnectToServer({
  initialUrl = "",
  initialAllowInvalidCertificates = false,
  embedded = false,
  onConnected,
}: {
  initialUrl?: string;
  initialAllowInvalidCertificates?: boolean;
  embedded?: boolean;
  onConnected: (connection: DesktopServerConnection) => void | Promise<void>;
}) {
  const [draft, setDraft] = React.useState(initialUrl);
  const [allowInvalidCertificates, setAllowInvalidCertificates] = React.useState(
    initialAllowInvalidCertificates,
  );
  const [error, setError] = React.useState<string | null>(null);
  const [isConnecting, setIsConnecting] = React.useState(false);

  React.useEffect(() => setDraft(initialUrl), [initialUrl]);
  React.useEffect(
    () => setAllowInvalidCertificates(initialAllowInvalidCertificates),
    [initialAllowInvalidCertificates],
  );

  const isHttpsDraft = draft.trim().toLowerCase().startsWith("https://");

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

        const connection = {
          baseUrl,
          allowInvalidCertificates:
            baseUrl.startsWith("https://") && allowInvalidCertificates,
        };

        setIsConnecting(true);
        const controller = new AbortController();
        const timeout = window.setTimeout(() => controller.abort(), 8_000);
        try {
          const transport = createDesktopHttpTransport(
            connection.allowInvalidCertificates,
          );
          const response = await transport.fetch(`${baseUrl}/health`, {
            signal: controller.signal,
          });
          if (!response.ok) {
            throw new Error(`The server returned ${response.status}.`);
          }
          const health = (await response.json()) as Partial<Health>;
          if (health.status !== "ok" || typeof health.core_version !== "string") {
            throw new Error("The address did not return a Loom health response.");
          }
          await desktopBaseUrlProvider.setConnection(connection);
          await onConnected(connection);
        } catch (connectionError) {
          setError(
            connectionErrorMessage(
              connectionError,
              baseUrl,
              connection.allowInvalidCertificates,
            ),
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
          onChange={(event) => {
            const value = event.target.value;
            setDraft(value);
            if (!value.trim().toLowerCase().startsWith("https://")) {
              setAllowInvalidCertificates(false);
            }
          }}
        />
      </div>

      <div className="flex items-start justify-between gap-4 rounded-md border p-3">
        <div className="flex flex-col gap-1">
          <Label htmlFor={embedded ? "settings-invalid-certs" : "invalid-certs"}>
            Allow invalid TLS certificates
          </Label>
          <p
            id={embedded ? "settings-invalid-certs-description" : "invalid-certs-description"}
            className="text-sm text-muted-foreground"
          >
            For HTTPS homelab servers using a self-signed certificate. Disabled by
            default.
          </p>
        </div>
        <Switch
          id={embedded ? "settings-invalid-certs" : "invalid-certs"}
          checked={isHttpsDraft && allowInvalidCertificates}
          disabled={!isHttpsDraft || isConnecting}
          aria-describedby={
            embedded ? "settings-invalid-certs-description" : "invalid-certs-description"
          }
          onCheckedChange={setAllowInvalidCertificates}
        />
      </div>

      {isHttpsDraft && allowInvalidCertificates ? (
        <Alert>
          <TriangleAlert aria-hidden="true" />
          <AlertTitle>Certificate verification reduced</AlertTitle>
          <AlertDescription>
            Loom will not verify this server&apos;s certificate chain or expiry.
            Hostname verification remains enabled. Use this only for a server you
            trust.
          </AlertDescription>
        </Alert>
      ) : null}

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
