import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Card, CardContent, CardHeader } from "@loom/ui-kit/components/ui/card";
import { ConnectorCard } from "@loom/ui-kit/components/ConnectorCard";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { ApiError, SessionExpiredError } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

const REFETCH_INTERVAL_MS = 10_000;

/** Platform-neutral connector dashboard; the host app supplies its shell. */
export function Dashboard({
  renderShell,
}: {
  renderShell: (content: React.ReactNode) => React.ReactNode;
}) {
  const api = useApiClient();
  const { isAuthenticated, signOut } = useAuth();
  const connectors = useQuery({
    queryKey: ["connectors"],
    queryFn: ({ signal }) => api.getConnectors(signal),
    enabled: isAuthenticated,
    refetchInterval: REFETCH_INTERVAL_MS,
    retry: (failureCount, error) =>
      !(error instanceof ApiError && error.isUnauthorized) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 2,
  });

  const isUnauthorized =
    connectors.error instanceof SessionExpiredError ||
    (connectors.error instanceof ApiError && connectors.error.isUnauthorized);

  React.useEffect(() => {
    if (isUnauthorized) void signOut();
  }, [isUnauthorized, signOut]);

  return renderShell(
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Connectors</h1>
        <p className="text-sm text-muted-foreground">
          Services Loom is managing, refreshed every {REFETCH_INTERVAL_MS / 1000}
          seconds.
        </p>
      </div>

      {connectors.isPending ? <ConnectorSkeletons /> : null}

      {connectors.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load connectors</AlertTitle>
          <AlertDescription>
            {describeConnectorError(connectors.error)}
          </AlertDescription>
        </Alert>
      ) : null}

      {connectors.isSuccess && connectors.data.length === 0 ? (
        <Alert>
          <AlertTitle>No connectors registered</AlertTitle>
          <AlertDescription>
            The backend reported an empty connector list.
          </AlertDescription>
        </Alert>
      ) : null}

      {connectors.isSuccess && connectors.data.length > 0 ? (
        <div className="grid gap-4 sm:grid-cols-2">
          {connectors.data.map((connector) => (
            <ConnectorCard
              key={connector.metadata.id}
              connector={connector}
              onActionComplete={() => void connectors.refetch()}
            />
          ))}
        </div>
      ) : null}
    </div>,
  );
}

function ConnectorSkeletons() {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {[0, 1].map((index) => (
        <Card key={index} className="surface-elevated">
          <CardHeader>
            <div className="flex items-start justify-between gap-3">
              <div className="flex flex-col gap-2">
                <Skeleton className="h-5 w-32" />
                <Skeleton className="h-4 w-40" />
              </div>
              <Skeleton className="h-5 w-16 rounded-full" />
            </div>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
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
