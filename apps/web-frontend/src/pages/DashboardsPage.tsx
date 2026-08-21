import { useQuery } from "@tanstack/react-query";
import { AlertCircle, LayoutDashboard, Plus } from "lucide-react";
import { Navigate, useNavigate, useParams } from "react-router-dom";

import { WebAppShell } from "@/components/WebAppShell";
import {
  DashboardCreateDialog,
  dashboardsQueryKey,
} from "@loom/ui-kit/components/DashboardSidebar";
import { DashboardView } from "@loom/ui-kit/components/DashboardView";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent } from "@loom/ui-kit/components/ui/card";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

export function DashboardsIndexPage() {
  const api = useApiClient();
  const navigate = useNavigate();
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
  });

  if (dashboards.isSuccess) {
    const destination =
      dashboards.data.find((dashboard) => dashboard.pinned) ??
      dashboards.data.find((dashboard) => dashboard.role === "owner");
    if (destination !== undefined) {
      return <Navigate to={`/dashboards/${destination.id}`} replace />;
    }
  }

  return (
    <WebAppShell>
      {dashboards.isPending ? (
        <div className="flex flex-col gap-4" aria-label="Loading dashboards">
          <Skeleton className="h-8 w-52" />
          <Skeleton className="h-48 w-full" />
        </div>
      ) : null}
      {dashboards.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load dashboards</AlertTitle>
          <AlertDescription>{describeConnectorError(dashboards.error)}</AlertDescription>
        </Alert>
      ) : null}
      {dashboards.isSuccess ? (
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
              onCreated={(dashboard) => navigate(`/dashboards/${dashboard.id}`)}
              trigger={
                <Button>
                  <Plus aria-hidden="true" />
                  Create your first dashboard
                </Button>
              }
            />
          </CardContent>
        </Card>
      ) : null}
    </WebAppShell>
  );
}

export function DashboardDetailPage() {
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();
  if (id === undefined) return <Navigate to="/dashboards" replace />;

  return (
    <WebAppShell>
      <DashboardView
        key={id}
        dashboardId={id}
        onDeleted={() => navigate("/dashboards", { replace: true })}
      />
    </WebAppShell>
  );
}
