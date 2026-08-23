import * as React from "react";
import { AlertCircle, TriangleAlert } from "lucide-react";

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
import type { HttpTransport } from "@loom/ui-kit/lib/api";

type Health = { status: string; core_version: string };

export type ServerConnection = {
  baseUrl: string;
  allowInvalidCertificates: boolean;
};

function normalizeServerUrl(value: string): string {
  const url = new URL(value.trim());
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Use an http:// or https:// server URL.");
  }
  if (url.username !== "" || url.password !== "") {
    throw new Error("The server URL must not contain credentials.");
  }
  if (url.search !== "" || url.hash !== "") {
    throw new Error("The server URL must not contain a query or fragment.");
  }
  return url.toString().replace(/\/+$/, "");
}

function connectionErrorMessage(
  error: unknown,
  baseUrl: string,
  allowInvalidCertificates: boolean,
  supportsInvalidCertificates: boolean,
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
    supportsInvalidCertificates &&
    baseUrl.startsWith("https://") &&
    !allowInvalidCertificates &&
    /load failed|certificate|tls|ssl/i.test(message)
  ) {
    return "The TLS connection failed. If this server uses a self-signed certificate, enable the certificate exception below and try again.";
  }
  return message;
}

/**
 * Runtime server selection shared by installed clients. Persistence stays in
 * the platform's `onConnected` callback, and native clients may inject a
 * transport factory when their TLS policy cannot be expressed by webview
 * `fetch`.
 */
export function ConnectToServer({
  initialUrl = "",
  initialAllowInvalidCertificates = false,
  embedded = false,
  supportsInvalidCertificates = false,
  invalidCertificateNote,
  getHttpTransport,
  onConnected,
}: {
  initialUrl?: string;
  initialAllowInvalidCertificates?: boolean;
  embedded?: boolean;
  supportsInvalidCertificates?: boolean;
  /** Platform-specific limits of the certificate exception, shown beside it. */
  invalidCertificateNote?: string;
  getHttpTransport?: (allowInvalidCertificates: boolean) => HttpTransport;
  onConnected: (connection: ServerConnection) => void | Promise<void>;
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
            supportsInvalidCertificates &&
            baseUrl.startsWith("https://") &&
            allowInvalidCertificates,
        };

        setIsConnecting(true);
        const controller = new AbortController();
        const timeout = window.setTimeout(() => controller.abort(), 8_000);
        try {
          const transport = getHttpTransport?.(
            connection.allowInvalidCertificates,
          ) ?? { fetch: globalThis.fetch.bind(globalThis) };
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
          await onConnected(connection);
        } catch (connectionError) {
          setError(
            connectionErrorMessage(
              connectionError,
              baseUrl,
              connection.allowInvalidCertificates,
              supportsInvalidCertificates,
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

      {supportsInvalidCertificates ? (
        <>
          <div className="flex items-start justify-between gap-4 rounded-md border p-3">
            <div className="flex flex-col gap-1">
              <Label htmlFor={embedded ? "settings-invalid-certs" : "invalid-certs"}>
                Allow invalid TLS certificates
              </Label>
              <p
                id={
                  embedded
                    ? "settings-invalid-certs-description"
                    : "invalid-certs-description"
                }
                className="text-sm text-muted-foreground"
              >
                For HTTPS homelab servers using a self-signed certificate. Disabled
                by default.
                {invalidCertificateNote === undefined ? null : (
                  <span className="mt-1 block">{invalidCertificateNote}</span>
                )}
              </p>
            </div>
            <Switch
              id={embedded ? "settings-invalid-certs" : "invalid-certs"}
              checked={isHttpsDraft && allowInvalidCertificates}
              disabled={!isHttpsDraft || isConnecting}
              aria-describedby={
                embedded
                  ? "settings-invalid-certs-description"
                  : "invalid-certs-description"
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
        </>
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
            Enter the address of the Loom server this app should manage.
          </CardDescription>
        </CardHeader>
        <CardContent>{form}</CardContent>
      </Card>
    </main>
  );
}
