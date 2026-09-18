import { Loader2, ServerCrash } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";

/** Shared wording for a held session waiting out an unreachable server. */
export const RECONNECTING_MESSAGE =
  "The server is unreachable. Loom keeps retrying and will not sign you out.";

export function BootScreen({
  baseUrl,
  title,
  message,
}: {
  baseUrl: string;
  /** Overrides the launch wording — "Reconnecting to Loom" during recovery. */
  title?: string;
  message?: string;
}) {
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <Loader2 className="h-8 w-8 animate-spin text-primary" aria-hidden="true" />
        <h1 className="text-lg font-semibold">
          {title ?? (baseUrl ? "Connecting to Loom" : "Starting Loom")}
        </h1>
        {baseUrl ? <p className="break-all text-sm text-muted-foreground">{baseUrl}</p> : null}
        {message ? <p className="text-sm text-muted-foreground">{message}</p> : null}
      </div>
    </main>
  );
}

export function BootErrorScreen({
  baseUrl,
  message,
  onRetry,
  onChangeServer,
  retrying = false,
}: {
  baseUrl: string;
  message: string;
  onRetry: () => void;
  onChangeServer?: () => void | Promise<void>;
  /**
   * Whether a retry is already scheduled. An unreachable server is a transient
   * condition the app recovers from on its own, so the screen says so rather
   * than implying the user must act — and it never offers to sign in, because
   * nothing here has rejected the stored session.
   */
  retrying?: boolean;
}) {
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <div className="flex w-full max-w-md flex-col gap-4">
        <Alert variant="destructive">
          <ServerCrash aria-hidden="true" />
          <AlertTitle>Could not connect to Loom</AlertTitle>
          <AlertDescription>
            <span className="block">{message}</span>
            <span className="mt-2 block break-all opacity-80">{baseUrl}</span>
          </AlertDescription>
        </Alert>
        {retrying ? (
          <p
            className="flex items-center justify-center gap-2 text-sm text-muted-foreground"
            role="status"
          >
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            Reconnecting automatically. You stay signed in.
          </p>
        ) : null}
        <div className="flex flex-col gap-2 sm:flex-row">
          <Button onClick={onRetry}>Try again</Button>
          {onChangeServer !== undefined ? (
            <Button variant="outline" onClick={() => void onChangeServer()}>
              Change server
            </Button>
          ) : null}
        </div>
      </div>
    </main>
  );
}
