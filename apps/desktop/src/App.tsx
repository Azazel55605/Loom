import * as React from "react";
import { AlertCircle } from "lucide-react";

import { ServerUrlField } from "@/components/ServerUrlField";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

/** Fallback when the user has not chosen a server yet. */
const DEFAULT_SERVER_URL = "http://localhost:8080";

/** localStorage key holding the user's chosen backend. */
const SERVER_URL_KEY = "loom.serverUrl";

/** Shape of the web-backend `/health` response. */
type Health = {
  status: string;
  core_version: string;
};

type State =
  | { kind: "loading" }
  | { kind: "ready"; health: Health }
  | { kind: "error"; message: string };

export default function App() {
  const [serverUrl, setServerUrl] = React.useState(
    () => localStorage.getItem(SERVER_URL_KEY) ?? DEFAULT_SERVER_URL,
  );
  const [state, setState] = React.useState<State>({ kind: "loading" });

  React.useEffect(() => {
    localStorage.setItem(SERVER_URL_KEY, serverUrl);

    const controller = new AbortController();
    setState({ kind: "loading" });

    fetch(`${serverUrl.replace(/\/+$/, "")}/health`, { signal: controller.signal })
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`${response.status} ${response.statusText}`);
        }
        return (await response.json()) as Health;
      })
      .then((health) => setState({ kind: "ready", health }))
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setState({
          kind: "error",
          message: error instanceof Error ? error.message : "Unknown error",
        });
      });

    return () => controller.abort();
  }, [serverUrl]);

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6">
      <Card className="w-full max-w-md">
        <CardHeader>
          <div className="flex items-center justify-between gap-4">
            <CardTitle>Loom</CardTitle>
            <Badge variant="secondary">desktop v{__APP_VERSION__}</Badge>
          </div>
          <CardDescription>
            Modular homelab management. Point this app at your Loom server.
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-5">
          <ServerUrlField
            value={serverUrl}
            onSubmit={setServerUrl}
            disabled={state.kind === "loading"}
          />

          {state.kind === "loading" && (
            <p className="text-sm text-muted-foreground" aria-live="polite">
              Connecting to {serverUrl}…
            </p>
          )}

          {state.kind === "error" && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertTitle>Backend unreachable</AlertTitle>
              <AlertDescription>{state.message}</AlertDescription>
            </Alert>
          )}

          {state.kind === "ready" && (
            <dl className="space-y-3">
              <div className="flex items-center justify-between gap-4">
                <dt className="text-sm text-muted-foreground">Backend status</dt>
                <dd>
                  <Badge
                    variant={state.health.status === "ok" ? "default" : "destructive"}
                  >
                    {state.health.status}
                  </Badge>
                </dd>
              </div>
              <div className="flex items-center justify-between gap-4">
                <dt className="text-sm text-muted-foreground">Backend core version</dt>
                <dd>
                  <Badge variant="outline">v{state.health.core_version}</Badge>
                </dd>
              </div>
            </dl>
          )}
        </CardContent>
      </Card>
    </main>
  );
}
