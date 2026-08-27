import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { ActionButtonWidget } from "@loom/ui-kit/widgets/ActionButton";
import { renderWidget } from "@loom/ui-kit/widgets/renderWidget";
import type {
  ConnectorError,
  ConnectorStatus,
  DashboardPlacement,
  PendingOperation,
} from "@loom/ui-kit/lib/api";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { useApiClient, useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import {
  matchesTarget,
  statusDetailsForTarget,
} from "@loom/ui-kit/lib/connector-details";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";
import { useRetainedStatusDetails } from "@loom/ui-kit/lib/use-retained-status-details";

type LiveReading = {
  status: ConnectorStatus | null;
  statusError?: ConnectorError;
  pendingOperation?: PendingOperation | null;
  diagnosis?: string | null;
};

export function ConnectorDetailModal({
  placement,
  open,
  onOpenChange,
}: {
  placement: DashboardPlacement;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const api = useApiClient();
  const socket = useConnectorStatusSocket();
  const { user } = useAuth();
  const instance = placement.connector;
  const [live, setLive] = React.useState<LiveReading | null>(null);
  const canControl = hasPermission(user?.permissions ?? [], PERMISSION_KEYS.connectorsControl);

  const detail = useQuery({
    queryKey: ["connector-instance", instance.id],
    queryFn: ({ signal }) => api.getConnectorInstance(instance.id, signal),
    enabled: open,
  });

  React.useEffect(() => {
    if (!open) {
      setLive(null);
      return;
    }
    return socket.subscribe([instance.id], (update) => {
      setLive({
        status: update.status,
        statusError: update.statusError,
        pendingOperation: update.pendingOperation,
        diagnosis: update.diagnosis,
      });
    });
  }, [instance.id, open, socket]);

  const execute = useMutation({
    mutationFn: ({ actionId, params }: { actionId: string; params: Record<string, unknown> }) =>
      api.executeConnectorAction(instance.id, actionId, params, placement.targetId),
    onSettled: () => void detail.refetch(),
  });
  const runAction = React.useCallback(async (actionId: string, params: Record<string, unknown>) => {
    const label = detail.data?.actions.find(
      (action) => matchesTarget(action, placement.targetId) && action.id === actionId,
    )?.label ?? actionId;
    try {
      const result = await execute.mutateAsync({ actionId, params });
      if (result.success) toast.success(`${instance.name}: ${label}`, { description: result.message });
      else toast.warning(`${instance.name}: ${label} declined`, { description: result.message });
      return result;
    } catch (error) {
      toast.error(`${instance.name}: ${label} failed`, { description: describeConnectorError(error) });
      throw error;
    }
  }, [detail.data, execute, instance.name, placement.targetId]);

  const reading: LiveReading = live ?? {
    status: detail.data?.status ?? instance.status,
    statusError: detail.data?.statusError ?? instance.statusError,
    pendingOperation: detail.data?.pendingOperation ?? instance.pendingOperation,
    diagnosis: detail.data?.diagnosis ?? instance.diagnosis,
  };
  const availability = connectorAvailability(reading);
  const rawDetails = statusDetailsForTarget(reading.status?.details, placement.targetId);
  const statusDetails = useRetainedStatusDetails(
    `${instance.id}:${placement.targetId ?? ""}`,
    rawDetails,
  );
  const targetDataPoints =
    detail.data?.dataPoints.filter((point) => matchesTarget(point, placement.targetId)) ?? [];
  const targetActions =
    detail.data?.actions.filter((action) => matchesTarget(action, placement.targetId)) ?? [];
  const boundActions = new Set(
    placement.widgetBindings.flatMap((binding) => "action" in binding ? [binding.action.actionId] : []),
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-5xl grid-rows-[auto_minmax(0,1fr)] overflow-hidden p-0 sm:h-auto sm:max-h-[90dvh] sm:w-[calc(100vw-3rem)]">
        <DialogHeader className="border-b px-5 py-4 pr-12 text-left">
          <div className="flex flex-wrap items-center gap-2">
            <ConnectorIcon
              typeIcon={instance.metadata.icon}
              iconOverride={instance.iconOverride}
              size={22}
            />
            <DialogTitle>
              {instance.name}
              {placement.targetId === null ? null : ` · ${placement.targetId}`}
            </DialogTitle>
            <Badge variant={availability.tone}>{availability.label}</Badge>
          </div>
          <DialogDescription>
            {instance.metadata.name} · {instance.connectorType} · v{instance.metadata.version}
          </DialogDescription>
          {/* Beneath the Badge that already said something is wrong. A plain
              line rather than a second Alert: one message per problem. */}
          {availability.diagnosis !== null ? (
            <p className="text-xs leading-relaxed text-muted-foreground">
              {availability.diagnosis}
            </p>
          ) : null}
          {reading.status !== null ? (
            <p className="text-xs text-muted-foreground">
              Last checked <time dateTime={reading.status.lastChecked}>{formatChecked(reading.status.lastChecked)}</time>
            </p>
          ) : null}
          {instance.displayFields.length > 0 ? (
            <dl className="grid gap-x-5 gap-y-1 pt-2 text-sm sm:grid-cols-[auto_1fr_auto_1fr]">
              {instance.displayFields.map((field) => <React.Fragment key={field.label}><dt className="text-muted-foreground">{field.label}</dt><dd className="truncate font-medium" title={field.value}>{field.value}</dd></React.Fragment>)}
            </dl>
          ) : null}
        </DialogHeader>

        <div className="min-h-0 overflow-y-auto px-5 py-4">
          {detail.isPending ? <ConnectorDetailSkeleton /> : detail.isError ? (
            <Alert variant="destructive"><AlertCircle aria-hidden="true" /><AlertDescription>{describeConnectorError(detail.error)}</AlertDescription></Alert>
          ) : (
            <div className="space-y-6">
              {reading.statusError !== undefined ? <Alert variant="destructive"><AlertCircle aria-hidden="true" /><AlertDescription>{describeConnectorError(reading.statusError)}</AlertDescription></Alert> : null}
              <div className="grid gap-5 md:grid-cols-2">
                {placement.widgetBindings.map((binding, index) => (
                  <React.Fragment key={index}>{renderWidget({ binding, statusDetails, dataPoints: targetDataPoints, actions: targetActions, onExecute: runAction, disabled: !canControl, unavailableReason: availability.unavailableReason, size: "expanded", className: "min-h-[5rem]" })}</React.Fragment>
                ))}
              </div>
              {targetActions.some((action) => !boundActions.has(action.id)) ? (
                <section className="space-y-3 border-t pt-5">
                  <h3 className="text-sm font-semibold">Other actions</h3>
                  <div className="grid gap-2 sm:grid-cols-2">
                    {targetActions.filter((action) => !boundActions.has(action.id)).map((action) => (
                      <ActionButtonWidget key={action.id} label={action.label} actionId={action.id} description={action.description} paramsSchema={action.paramsSchema} config={{}} onExecute={runAction} disabled={!canControl || availability.actionsDisabled} />
                    ))}
                  </div>
                </section>
              ) : null}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ConnectorDetailSkeleton() {
  return <div className="space-y-5"><div className="grid gap-3 sm:grid-cols-2"><Skeleton className="h-8" /><Skeleton className="h-8" /></div><Skeleton className="h-48 w-full" /><Skeleton className="h-24 w-full" /></div>;
}

function formatChecked(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
}
