import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Grid2X2, Pencil, Share2, Trash2 } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@loom/ui-kit/components/ui/alert-dialog";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@loom/ui-kit/components/ui/tooltip";
import { DashboardSharesDialog } from "@loom/ui-kit/components/DashboardSharesDialog";
import { dashboardsQueryKey } from "@loom/ui-kit/components/DashboardSidebar";
import type {
  ConnectorInstanceSummary,
  DashboardSummary,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

const dashboardQueryKey = (dashboardId: string) => ["dashboard", dashboardId] as const;

/** Dashboard detail shell. Grid placement and widgets intentionally follow later. */
export function DashboardView({
  dashboardId,
  onDeleted,
}: {
  dashboardId: string;
  onDeleted: () => void;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [editingName, setEditingName] = React.useState(false);
  const [name, setName] = React.useState("");
  const [sharesOpen, setSharesOpen] = React.useState(false);
  const [deleteOpen, setDeleteOpen] = React.useState(false);

  const dashboard = useQuery({
    queryKey: dashboardQueryKey(dashboardId),
    queryFn: ({ signal }) => api.getDashboard(dashboardId, signal),
  });

  React.useEffect(() => {
    if (!editingName && dashboard.data !== undefined) setName(dashboard.data.name);
  }, [dashboard.data, editingName]);

  const rename = useMutation({
    mutationFn: () => api.renameDashboard(dashboardId, name),
    onSuccess: (updated) => {
      queryClient.setQueryData(dashboardQueryKey(dashboardId), updated);
      queryClient.setQueryData<DashboardSummary[]>(dashboardsQueryKey, (current) =>
        current?.map((item) =>
          item.id === dashboardId ? { ...item, name: updated.name } : item,
        ),
      );
      setEditingName(false);
    },
  });
  const remove = useMutation({
    mutationFn: () => api.deleteDashboard(dashboardId),
    onSuccess: async () => {
      queryClient.removeQueries({ queryKey: dashboardQueryKey(dashboardId) });
      await queryClient.invalidateQueries({ queryKey: dashboardsQueryKey });
      setDeleteOpen(false);
      onDeleted();
    },
  });

  if (dashboard.isPending) return <DashboardViewSkeleton />;
  if (dashboard.isError) {
    return (
      <Alert variant="destructive">
        <AlertCircle aria-hidden="true" />
        <AlertTitle>Could not load dashboard</AlertTitle>
        <AlertDescription>{describeConnectorError(dashboard.error)}</AlertDescription>
      </Alert>
    );
  }

  const detail = dashboard.data;
  const isOwner = detail.role === "owner";

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-stretch justify-between gap-4 sm:flex-row sm:items-start">
        <div className="min-w-0 flex-1">
          {editingName ? (
            <form
              className="flex max-w-xl items-center gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                if (name.trim()) rename.mutate();
              }}
            >
              <Input
                aria-label="Dashboard name"
                autoFocus
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
              <Button type="submit" size="sm" disabled={!name.trim() || rename.isPending}>
                {rename.isPending ? "Saving…" : "Save"}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  setEditingName(false);
                  setName(detail.name);
                  rename.reset();
                }}
              >
                Cancel
              </Button>
            </form>
          ) : (
            <div className="flex min-w-0 items-center gap-2">
              <h1 className="truncate text-2xl font-semibold tracking-tight">{detail.name}</h1>
              {isOwner ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  aria-label={`Rename ${detail.name}`}
                  onClick={() => setEditingName(true)}
                >
                  <Pencil aria-hidden="true" />
                </Button>
              ) : null}
            </div>
          )}
          <div className="mt-2 flex items-center gap-2">
            <Badge variant="outline" className="capitalize">
              {detail.role}
            </Badge>
            <span className="text-sm text-muted-foreground">
              Owned by {detail.owner.username}
            </span>
          </div>
          {rename.isError ? (
            <Alert variant="destructive" className="mt-3 max-w-xl">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(rename.error)}</AlertDescription>
            </Alert>
          ) : null}
        </div>

        {isOwner ? (
          <div className="flex items-center justify-end gap-2">
            <Button type="button" variant="outline" size="sm" onClick={() => setSharesOpen(true)}>
              <Share2 aria-hidden="true" />
              Share
            </Button>
            <Button type="button" variant="destructive" size="sm" onClick={() => setDeleteOpen(true)}>
              <Trash2 aria-hidden="true" />
              Delete
            </Button>
          </div>
        ) : null}
      </div>

      <Alert>
        <Grid2X2 aria-hidden="true" />
        <AlertTitle>Grid and widgets are coming next</AlertTitle>
        <AlertDescription>
          Placements are shown as a read-only list for now while the dashboard layout system is built.
        </AlertDescription>
      </Alert>

      {detail.placements.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <p className="font-medium">No connectors placed on this dashboard yet.</p>
            <p className="max-w-md text-sm text-muted-foreground">
              Placement editing will arrive with the dashboard grid and widget system.
            </p>
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span tabIndex={0}>
                    <Button type="button" disabled>
                      Add connector
                    </Button>
                  </span>
                </TooltipTrigger>
                <TooltipContent>Placement editing is coming in the next update.</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-4">
          {detail.placements.map((placement) => (
            <PlacementSummaryCard key={placement.id} connector={placement.connector} />
          ))}
        </div>
      )}

      {isOwner ? (
        <DashboardSharesDialog
          dashboardId={detail.id}
          dashboardName={detail.name}
          open={sharesOpen}
          onOpenChange={setSharesOpen}
        />
      ) : null}

      <AlertDialog
        open={deleteOpen}
        onOpenChange={(next) => {
          setDeleteOpen(next);
          if (!next) remove.reset();
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {detail.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently removes the dashboard, its shares, pins, and placements.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {remove.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(remove.error)}</AlertDescription>
            </Alert>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={remove.isPending}
              onClick={(event) => {
                event.preventDefault();
                remove.mutate();
              }}
            >
              {remove.isPending ? "Deleting…" : "Delete dashboard"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function PlacementSummaryCard({ connector }: { connector: ConnectorInstanceSummary }) {
  const health = connector.status?.health ?? "unknown";
  return (
    <Card>
      <CardHeader className="flex-row items-start justify-between space-y-0">
        <div className="min-w-0">
          <CardTitle className="truncate text-base">{connector.name}</CardTitle>
          <CardDescription>{connector.metadata.name}</CardDescription>
        </div>
        <Badge variant={health} className="capitalize">
          {connector.status === null ? "No reading" : health}
        </Badge>
      </CardHeader>
      <CardContent>
        {connector.displayFields.length > 0 ? (
          <dl className="grid gap-2 sm:grid-cols-2">
            {connector.displayFields.map((field) => (
              <div key={field.label} className="surface-panel rounded-md border p-3">
                <dt className="text-xs text-muted-foreground">{field.label}</dt>
                <dd className="mt-1 text-sm font-medium">{field.value}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-sm text-muted-foreground">No summary fields reported.</p>
        )}
      </CardContent>
    </Card>
  );
}

function DashboardViewSkeleton() {
  return (
    <div className="flex flex-col gap-6" aria-label="Loading dashboard">
      <div className="flex items-center justify-between gap-4">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-9 w-36" />
      </div>
      <Skeleton className="h-20 w-full" />
      <Skeleton className="h-36 w-full" />
    </div>
  );
}
