import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Loader2, Plug, Plus } from "lucide-react";
import { toast } from "sonner";

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
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent, CardHeader } from "@loom/ui-kit/components/ui/card";
import { ConnectorCard } from "@loom/ui-kit/components/ConnectorCard";
import { ConnectorInstanceDialog } from "@loom/ui-kit/components/ConnectorInstanceDialog";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorInstanceDetail,
  type ConnectorInstanceSummary,
} from "@loom/ui-kit/lib/api";
import { useApiClient, useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";

/**
 * The connector instances this deployment has, one card each.
 *
 * **This is a placeholder for the real dashboard, not the real dashboard.** The
 * eventual product is a grid of user-placed widgets — `defaultLayout` bound to
 * `dataPoints`, rendered as gauges, charts and status dots, arranged per user
 * and persisted server-side. Neither the placement storage nor the widget
 * primitives exist yet, and both are deliberate follow-ups.
 *
 * What this does instead is the honest intermediate: list every instance, show
 * its status and its `displayFields`, and offer a plain button per action. That
 * keeps every connector visible and operable while the widget system is built,
 * and it is the thing to delete — not extend — once that lands.
 *
 * The component is platform-neutral; the host app supplies its shell.
 */

export function ConnectorsView({
  renderShell,
}: {
  renderShell: (content: React.ReactNode) => React.ReactNode;
}) {
  const api = useApiClient();
  const connectorSocket = useConnectorStatusSocket();
  const queryClient = useQueryClient();
  const { isAuthenticated, signOut, user } = useAuth();

  // Visibility only. **Not a security boundary** — the backend requires a
  // global `connectors.manage` grant on every one of these routes and answers
  // 403 regardless of what the UI chose to render. See lib/permissions.ts.
  const canManage = hasPermission(user?.permissions ?? [], PERMISSION_KEYS.connectorsManage);

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: isAuthenticated,
    retry: (failureCount, error) =>
      !(error instanceof ApiError && error.isUnauthorized) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 2,
  });

  const instanceIds = React.useMemo(
    () => instances.data?.map((instance) => instance.id) ?? [],
    [instances.data],
  );

  React.useEffect(() => {
    if (!isAuthenticated || instanceIds.length === 0) return;

    return connectorSocket.subscribe(instanceIds, (update) => {
      queryClient.setQueryData<ConnectorInstanceSummary[]>(
        ["connector-instances"],
        (current) =>
          current?.map((instance) =>
            instance.id === update.instanceId
              ? {
                  ...instance,
                  status: update.status,
                  statusError: update.statusError,
                }
              : instance,
          ),
      );
      queryClient.setQueryData<ConnectorInstanceDetail>(
        ["connector-instance", update.instanceId],
        (current) =>
          current === undefined
            ? undefined
            : {
                ...current,
                status: update.status,
                statusError: update.statusError,
              },
      );
    });
  }, [connectorSocket, instanceIds, isAuthenticated, queryClient]);

  const [createOpen, setCreateOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<ConnectorInstanceSummary | null>(null);
  const [deleting, setDeleting] = React.useState<ConnectorInstanceSummary | null>(null);

  const invalidate = React.useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ["connector-instances"] });
  }, [queryClient]);

  const removeInstance = useMutation({
    mutationFn: (target: ConnectorInstanceSummary) => api.deleteConnectorInstance(target.id),
    onSuccess: async (_result, target) => {
      setDeleting(null);
      toast.success(`Deleted ${target.name}.`);
      // Order matters. The list refetch is what unmounts the card, and the card
      // is what observes the per-instance detail query. Dropping that query
      // first would make its still-mounted observer immediately refetch a
      // deleted id — a pointless request that answers 404 and logs an error in
      // the console. Refresh the list, let the card go, then discard the
      // orphaned cache entry.
      await invalidate();
      queryClient.removeQueries({ queryKey: ["connector-instance", target.id] });
    },
    onError: (error: unknown) => {
      const failure = describeAdminFailure(error);
      toast.error(
        failure.kind === "refused" ? "That deletion was refused" : "Could not delete connector",
        { description: failure.message, duration: 10_000 },
      );
    },
  });

  const isUnauthorized =
    instances.error instanceof SessionExpiredError ||
    (instances.error instanceof ApiError && instances.error.isUnauthorized);

  React.useEffect(() => {
    if (isUnauthorized) void signOut();
  }, [isUnauthorized, signOut]);

  const isEmpty = instances.isSuccess && instances.data.length === 0;

  return renderShell(
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Connectors</h1>
          <p className="text-sm text-muted-foreground">
            Services Loom is managing, with live status updates.
          </p>
        </div>

        {/* Rendered only for a user who could actually succeed. A disabled
            button would invite the question without answering it, and the
            answer — "ask for connectors.manage" — is not something a tooltip
            carries well. */}
        {canManage && (
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus aria-hidden="true" />
            Add connector
          </Button>
        )}
      </div>

      {instances.isPending ? <ConnectorSkeletons /> : null}

      {instances.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load connectors</AlertTitle>
          <AlertDescription>{describeConnectorError(instances.error)}</AlertDescription>
        </Alert>
      ) : null}

      {isEmpty ? <EmptyState canManage={canManage} onAdd={() => setCreateOpen(true)} /> : null}

      {instances.isSuccess && instances.data.length > 0 ? (
        <div className="grid gap-4 sm:grid-cols-2">
          {instances.data.map((instance) => (
            <ConnectorCard
              key={instance.id}
              instance={instance}
              onEdit={canManage ? setEditing : undefined}
              onDelete={canManage ? setDeleting : undefined}
            />
          ))}
        </div>
      ) : null}

      <ConnectorInstanceDialog
        // Remounted per target so the generated form starts from the right
        // values instead of carrying the previous instance's state over.
        key={editing?.id ?? "create"}
        open={createOpen || editing !== null}
        instance={editing}
        onOpenChange={(open) => {
          if (!open) {
            setCreateOpen(false);
            setEditing(null);
          }
        }}
        onSaved={invalidate}
      />

      <AlertDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {deleting?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              Loom stops managing this service and forgets its configuration. The service
              itself is not affected. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removeInstance.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={removeInstance.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (deleting !== null) removeInstance.mutate(deleting);
              }}
            >
              {removeInstance.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>,
  );
}

/**
 * What a fresh instance looks like.
 *
 * A blank screen here would be indistinguishable from a broken one — connectors
 * are no longer shipped by default, so zero of them is the normal first state
 * and needs to read as an invitation rather than as an absence.
 */
function EmptyState({ canManage, onAdd }: { canManage: boolean; onAdd: () => void }) {
  return (
    <Card className="surface-elevated">
      <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
        <Plug className="h-8 w-8 text-muted-foreground" aria-hidden="true" />
        <div className="space-y-1">
          <p className="font-medium">No connectors yet</p>
          <p className="max-w-md text-sm text-muted-foreground">
            {canManage
              ? "A connector is Loom's link to one service it manages. Add one to see its status and act on it."
              : "Nothing has been added to this instance yet. Someone with the connectors.manage permission can add one."}
          </p>
        </div>
        {canManage && (
          <Button onClick={onAdd}>
            <Plus aria-hidden="true" />
            Add connector
          </Button>
        )}
      </CardContent>
    </Card>
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
