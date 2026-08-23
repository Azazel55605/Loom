import { Loader2, ServerCrash } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";

export function BootScreen({ baseUrl }: { baseUrl: string }) {
  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <div className="flex max-w-md flex-col items-center gap-3 text-center">
        <Loader2 className="h-8 w-8 animate-spin text-primary" aria-hidden="true" />
        <h1 className="text-lg font-semibold">{baseUrl ? "Connecting to Loom" : "Starting Loom"}</h1>
        {baseUrl ? <p className="break-all text-sm text-muted-foreground">{baseUrl}</p> : null}
      </div>
    </main>
  );
}

export function BootErrorScreen({
  baseUrl,
  message,
  onRetry,
  onChangeServer,
}: {
  baseUrl: string;
  message: string;
  onRetry: () => void;
  onChangeServer?: () => void | Promise<void>;
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
