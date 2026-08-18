import * as React from "react";
import { AlertCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

/**
 * Base URL of the web-backend API.
 *
 * Relative by default: the frontend's own server proxies `/api` to the backend,
 * so requests stay same-origin and no backend host is compiled into the bundle.
 * That is what lets one published image work for every deployment — an absolute
 * URL baked in at build time can only ever be correct for whoever built it.
 * See docs/adr/0006-frontend-api-same-origin.md.
 *
 * `VITE_API_URL` overrides it with an absolute URL for setups without that
 * proxy; those requests are cross-origin and rely on the backend's CORS policy.
 */
const API_URL = resolveApiUrl();

/**
 * `??` is deliberately not used: a declared-but-empty `VITE_API_URL` (which is
 * what an unset Docker build arg produces) arrives as `""`, and `""` is neither
 * null nor undefined, so it would silently win and every request would go to
 * the wrong path.
 */
function resolveApiUrl(): string {
  const configured = import.meta.env.VITE_API_URL?.trim();
  return configured ? configured : "/api";
}

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
  const [state, setState] = React.useState<State>({ kind: "loading" });

  React.useEffect(() => {
    const controller = new AbortController();

    fetch(`${API_URL}/health`, { signal: controller.signal })
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
  }, []);

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6">
      <Card className="w-full max-w-md">
        <CardHeader>
          <div className="flex items-center justify-between gap-4">
            <CardTitle>Loom</CardTitle>
            <Badge variant="secondary">frontend v{__APP_VERSION__}</Badge>
          </div>
          <CardDescription>
            Modular homelab management. Connected to {API_URL === "/api" ? "this host" : API_URL}
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-4">
          {state.kind === "loading" && (
            <div className="space-y-3" aria-busy="true" aria-live="polite">
              <Skeleton className="h-5 w-2/3" />
              <Skeleton className="h-5 w-1/2" />
            </div>
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
                  <Badge variant={state.health.status === "ok" ? "default" : "destructive"}>
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
