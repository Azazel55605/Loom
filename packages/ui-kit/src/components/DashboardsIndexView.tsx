import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle, LayoutDashboard, Plus } from "lucide-react";

import {
  DashboardCreateDialog,
  dashboardsQueryKey,
} from "@loom/ui-kit/components/DashboardSidebar";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent } from "@loom/ui-kit/components/ui/card";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/** Shared dashboard landing state; each client supplies its navigation adapter. */
export function DashboardsIndexView({
  onNavigate,
}: {
  onNavigate: (dashboardId: string) => void;
}) {
  const api = useApiClient();
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
  });

  // Hidden dashboards are skipped when choosing where to land. One is usually
  // the destination of a button tile rather than somewhere to start, and a
  // dashboard deliberately kept out of the sidebar becoming somebody's home
  // page purely because it sorted first would be the opposite of what the flag
  // was set for. It remains reachable by id, and by anything pointing at it.
  const landable = dashboards.data?.filter((dashboard) => !dashboard.hidden);
  const destination =
    landable?.find((dashboard) => dashboard.pinned) ??
    landable?.find((dashboard) => dashboard.role === "owner");
  const destinationId = destination?.id;
  const onNavigateRef = React.useRef(onNavigate);
  onNavigateRef.current = onNavigate;

  React.useEffect(() => {
    if (destinationId !== undefined) onNavigateRef.current(destinationId);
  }, [destinationId]);

  if (destinationId !== undefined) return null;

  if (dashboards.isPending) {
    return (
      <div className="flex flex-col gap-4" aria-label="Loading dashboards">
        <Skeleton className="h-8 w-52" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (dashboards.isError) {
    return (
      <Alert variant="destructive">
        <AlertCircle aria-hidden="true" />
        <AlertTitle>Could not load dashboards</AlertTitle>
        <AlertDescription>{describeConnectorError(dashboards.error)}</AlertDescription>
      </Alert>
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col items-center gap-4 py-14 text-center">
        <LayoutDashboard aria-hidden="true" className="h-10 w-10 text-muted-foreground" />
        <div>
          <h1 className="text-xl font-semibold">Create your first dashboard</h1>
          <p className="mt-1 max-w-md text-sm text-muted-foreground">
            Dashboards give your connectors a focused home. Shared dashboards remain
            available in the sidebar.
          </p>
        </div>
        <DashboardCreateDialog
          onCreated={(dashboard) => onNavigate(dashboard.id)}
          trigger={
            <Button>
              <Plus aria-hidden="true" />
              Create your first dashboard
            </Button>
          }
        />
      </CardContent>
    </Card>
  );
}
