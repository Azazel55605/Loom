import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { AppShell } from "@/components/AppShell";
import { ConnectorCard } from "@/components/ConnectorCard";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ApiError, getConnectors, SessionExpiredError } from "@/lib/api";
import { useAuth } from "@/lib/auth-context";
import { describeConnectorError } from "@/lib/connector-error";

/**
 * How often the connector list refetches.
 *
 * Polling is the deliberate choice for now: it is a few lines against a stable
 * endpoint, and a homelab dashboard does not need sub-second status. Pushing
 * status over a WebSocket or SSE is the expected upgrade once there is a real
 * connector whose state changes faster than this, and it is a backend change
 * before it is a frontend one.
 */
const REFETCH_INTERVAL_MS = 10_000;

export function DashboardPage() {
  const { isAuthenticated, signOut } = useAuth();

  const connectors = useQuery({
    queryKey: ["connectors"],
    queryFn: ({ signal }) => getConnectors(signal),
    enabled: isAuthenticated,
    refetchInterval: REFETCH_INTERVAL_MS,
    // The client already refreshes once on a 401, so a 401 reaching here means
    // the session is genuinely over and retrying cannot help. Same for an
    // expired refresh token.
    retry: (failureCount, error) =>
      !(error instanceof ApiError && error.isUnauthorized) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 2,
  });

  // A 401 means the session ended underneath us — the token expired, or the
  // backend restarted. Clearing it re-renders the guard, which redirects to
  // the login screen. Done in an effect rather than during render: `signOut`
  // updates state in a parent provider, and doing that mid-render is exactly
  // the pattern React warns about.
  const isUnauthorized =
    connectors.error instanceof SessionExpiredError ||
    (connectors.error instanceof ApiError && connectors.error.isUnauthorized);
  React.useEffect(() => {
    if (isUnauthorized) void signOut();
  }, [isUnauthorized, signOut]);

  return (
    <AppShell>
      <div className="space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Connectors</h1>
          <p className="text-sm text-muted-foreground">
            Services Loom is managing, refreshed every{" "}
            {REFETCH_INTERVAL_MS / 1000} seconds.
          </p>
        </div>

        {connectors.isPending && <ConnectorSkeletons />}

        {connectors.isError && (
          <Alert variant="destructive">
            <AlertCircle className="h-4 w-4" aria-hidden="true" />
            <AlertTitle>Could not load connectors</AlertTitle>
            <AlertDescription>
              {describeConnectorError(connectors.error)}
            </AlertDescription>
          </Alert>
        )}

        {connectors.isSuccess && connectors.data.length === 0 && (
          <Alert>
            <AlertTitle>No connectors registered</AlertTitle>
            <AlertDescription>
              The backend reported an empty connector list.
            </AlertDescription>
          </Alert>
        )}

        {connectors.isSuccess && connectors.data.length > 0 && (
          <div className="grid gap-4 sm:grid-cols-2">
            {connectors.data.map((connector) => (
              <ConnectorCard
                key={connector.metadata.id}
                connector={connector}
                onActionComplete={() => void connectors.refetch()}
              />
            ))}
          </div>
        )}
      </div>
    </AppShell>
  );
}

/** Placeholder cards matching the real ones' shape, so the layout does not jump
 *  when the data lands. */
function ConnectorSkeletons() {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {[0, 1].map((index) => (
        <Card key={index} className="surface-elevated">
          <CardHeader>
            <div className="flex items-start justify-between gap-3">
              <div className="space-y-2">
                <Skeleton className="h-5 w-32" />
                <Skeleton className="h-4 w-40" />
              </div>
              <Skeleton className="h-5 w-16 rounded-full" />
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            <Skeleton className="h-4 w-28" />
            <div className="flex gap-2">
              <Skeleton className="h-8 w-20" />
              <Skeleton className="h-8 w-16" />
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
