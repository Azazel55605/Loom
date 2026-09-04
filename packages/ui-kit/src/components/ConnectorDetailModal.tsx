import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Layers } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { InstanceActionHistory } from "@loom/ui-kit/components/InstanceActionHistory";
import { ResourceKindBrowser } from "@loom/ui-kit/components/ResourceKindBrowser";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@loom/ui-kit/components/ui/tabs";
import { ConnectorStatusBadge } from "@loom/ui-kit/components/ConnectorStatusBadge";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { ActionButtonWidget } from "@loom/ui-kit/widgets/ActionButton";
import { renderWidget } from "@loom/ui-kit/widgets/renderWidget";
import type {
  ConnectorError,
  ConnectorStatus,
  DashboardPlacement,
  PendingOperation,
} from "@loom/ui-kit/lib/api";
import { appliesToTarget } from "@loom/ui-kit/lib/api";
import { describeTarget } from "@loom/ui-kit/lib/target-label";
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

/** How long the dialog's open animation needs before its body can be scrolled
 *  meaningfully. Comfortably longer than the 150ms transition it is waiting on;
 *  a scroll that lands late is invisible, one that lands early does nothing. */
const DIALOG_OPEN_SETTLE_MS = 220;

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
  focus = null,
  initialTab = null,
}: {
  placement: DashboardPlacement;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Which widget the modal was opened *for*, when it was opened for one.
   *
   * `"logs"` comes from a log preview's expand affordance: the reader asked to
   * see that log, and on a placement with eight other widgets the pane they
   * asked for may be below the fold. Deliberately one narrow enum rather than a
   * general "scroll to binding N" API — this is the only case that exists, and
   * a selector taking indices would encode a placement's binding order into its
   * caller.
   */
  focus?: "logs" | null;
  /** Opens the universal History tab when arriving from the global audit log. */
  initialTab?: "history" | null;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
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
  const subTargets = useQuery({
    queryKey: ["connector-instance-sub-targets", instance.id],
    queryFn: ({ signal }) => api.getSubTargets(instance.id, signal),
    enabled: open && placement.targetId !== null,
    staleTime: 30_000,
  });
  const targetMetadata = subTargets.data?.find((target) => target.id === placement.targetId);
  const target = targetMetadata === undefined
    ? describeTarget(placement.targetId)
    : { text: targetMetadata.label, isStack: targetMetadata.kind === "stack" };

  // Fetched only while the modal is open: a browsable kind list is of no use
  // to a closed dialog, and a dashboard of twenty tiles would otherwise ask
  // twenty connectors what they can browse before anyone had looked at one.
  const resourceKinds = useQuery({
    // Keyed by target as well as instance: what a connector publishes can
    // depend on which view is open — a Docker stack has a Members table its
    // containers do not — so two targets of one instance are two answers.
    queryKey: ["connector-resource-kinds", instance.id, placement.targetId],
    queryFn: ({ signal }) => api.getResourceKinds(instance.id, placement.targetId, signal),
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
    onSettled: async () => {
      await Promise.all([
        detail.refetch(),
        queryClient.invalidateQueries({ queryKey: ["connector-action-log", instance.id] }),
        queryClient.invalidateQueries({ queryKey: ["global-audit-log"] }),
      ]);
    },
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
  // The first log pane on this placement, so the "logs" focus has something to
  // aim at. Indexed rather than ref-per-widget: `renderWidget` returns one
  // element per binding and the wrapper around it is the only thing this
  // component owns.
  const focusIndex = placement.widgetBindings.findIndex(
    (binding) => "display" in binding && binding.display.widgetType === "logStream",
  );

  // A **callback ref**, not an effect reading a ref object, and that is not a
  // style preference. Radix renders the dialog's contents into a portal that
  // commits *after* this component's own effects, so an effect that reaches
  // for the pane at open time finds `null` every single time and scrolls
  // nothing. Doing the work when the node itself attaches is the only ordering
  // that is actually guaranteed.
  const focusPending = React.useRef(false);
  const scrollTimer = React.useRef(0);
  React.useEffect(() => {
    focusPending.current = open && focus === "logs";
  }, [open, focus]);
  React.useEffect(() => () => window.clearTimeout(scrollTimer.current), []);
  const focusRef = React.useCallback((node: HTMLDivElement | null) => {
    if (node === null || !focusPending.current) return;
    // One shot per open: re-scrolling on every re-render would fight a reader
    // who has since scrolled somewhere else.
    focusPending.current = false;
    // *After* the dialog's open animation, not on the same tick as it. A
    // dialog still growing into place has a body with no scrollable extent
    // yet, so an immediate scroll silently does nothing.
    //
    // `block: "start"` rather than `"nearest"`: the reader asked for this pane
    // specifically, and "nearest" satisfies itself with one line of it showing
    // at the bottom edge.
    scrollTimer.current = window.setTimeout(() => {
      window.requestAnimationFrame(() => node.scrollIntoView({ block: "start" }));
    }, DIALOG_OPEN_SETTLE_MS);
  }, []);

  // Only the kinds that mean something at this altitude. A container's modal
  // must not offer "Images" — the daemon's image list is not a smaller thing
  // when you are looking at one container, it is a different question — and
  // the host's modal must not offer a kind that only exists per target. The
  // connector declares which, so nothing here knows what any of them are.
  const browsableKinds = (resourceKinds.data ?? []).filter((kind) =>
    appliesToTarget(kind, placement.targetId),
  );
  // Do not choose History merely because the descriptor query has not settled
  // yet. Existing connectors should continue opening on their first resource
  // tab; the global audit-log deep link is the one deliberate exception.
  const defaultDetailTab =
    initialTab === "history"
      ? "history"
      : resourceKinds.isPending
        ? null
        : (browsableKinds[0]?.kind ?? "history");

  // A sub-target picked from a host resource table. The nested modal below is
  // the same component a dashboard tile opens, so there is one detail view.
  // Host-only tables cannot appear again inside their target modal, so this
  // cannot recurse.
  const [nestedTarget, setNestedTarget] = React.useState<{
    targetId: string;
    focusLogs: boolean;
  } | null>(null);
  React.useEffect(() => {
    if (!open) setNestedTarget(null);
  }, [open]);

  const boundActions = new Set(
    placement.widgetBindings.flatMap((binding) => "action" in binding ? [binding.action.actionId] : []),
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-5xl grid-rows-[auto_minmax(0,1fr)] overflow-hidden p-0 sm:h-auto sm:max-h-[90dvh] sm:w-[calc(100vw-3rem)]">
        <DialogHeader className="border-b px-5 py-4 pr-12 text-left">
          <div className="flex flex-wrap items-center gap-2">
            {/* The same icon rule the tile follows, so opening a stack does not
                change what it looks like. */}
            {targetMetadata?.icon !== undefined ? (
              <ConnectorIcon typeIcon={targetMetadata.icon} iconOverride={null} size={22} />
            ) : target?.isStack === true ? (
              <Layers className="shrink-0 text-muted-foreground" size={22} aria-hidden="true" />
            ) : (
              <ConnectorIcon
                typeIcon={instance.metadata.icon}
                iconOverride={instance.iconOverride}
                size={22}
              />
            )}
            <DialogTitle>
              {instance.name}
              {target === null ? null : ` · ${target.text}`}
            </DialogTitle>
            <ConnectorStatusBadge availability={availability} />
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
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                {placement.widgetBindings.map((binding, index) => (
                  <div
                    key={index}
                    ref={index === focusIndex ? focusRef : undefined}
                    className={expandedWidgetSpansRow(binding)
                      ? "min-w-0 sm:col-span-2 lg:col-span-3"
                      : "min-w-0"}
                  >
                    {renderWidget({ binding, statusDetails, dataPoints: targetDataPoints, actions: targetActions, onExecute: runAction, disabled: !canControl, unavailableReason: availability.unavailableReason, size: "expanded", className: "min-h-[5rem]" })}
                  </div>
                ))}
              </div>
              {/* Resource kinds remain descriptor-driven; History is universal
                  because action logging happens in the platform endpoint for
                  every connector rather than being a connector capability. */}
              <section className="flex flex-col gap-3 border-t pt-5">
                {defaultDetailTab === null ? (
                  <Skeleton className="h-9 w-full" />
                ) : (
                  <Tabs
                    defaultValue={defaultDetailTab}
                  >
                    <div className="max-w-full overflow-x-auto pb-1">
                      <TabsList className="w-max min-w-full justify-start sm:min-w-0">
                        {browsableKinds.map((kind) => (
                          <TabsTrigger key={kind.kind} value={kind.kind}>
                            {kind.label}
                          </TabsTrigger>
                        ))}
                        <TabsTrigger value="history">History</TabsTrigger>
                      </TabsList>
                    </div>
                    {browsableKinds.map((kind) => (
                      <TabsContent key={kind.kind} value={kind.kind} className="pt-3">
                        <ResourceKindBrowser
                          instanceId={instance.id}
                          targetId={placement.targetId}
                          descriptor={kind}
                          onOpenTarget={(targetId) =>
                            setNestedTarget({ targetId, focusLogs: kind.kind === "logs" })
                          }
                          // The same rule the widgets follow: a viewer without
                          // `connectors.control`, or a connector that cannot be
                          // reached, gets buttons that say why before the click.
                          disabled={!canControl || availability.actionsDisabled}
                          disabledReason={
                            canControl
                              ? availability.unavailableReason
                              : "You do not have permission to control this connector."
                          }
                        />
                      </TabsContent>
                    ))}
                    <TabsContent value="history" className="pt-3">
                      <InstanceActionHistory
                        instanceId={instance.id}
                        actions={detail.data?.actions ?? []}
                      />
                    </TabsContent>
                  </Tabs>
                )}
                </section>

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
      {nestedTarget === null ? null : (
        <ConnectorDetailModal
          placement={targetPlacement(placement, nestedTarget.targetId, nestedTarget.focusLogs)}
          open
          onOpenChange={(next) => {
            if (!next) setNestedTarget(null);
          }}
          focus={nestedTarget.focusLogs ? "logs" : null}
        />
      )}
    </Dialog>
  );
}

/**
 * A stand-in placement for one sub-target, so its detail view can be opened
 * from somewhere that is not a dashboard.
 *
 * **Not a real placement, and not saved.** The rows of a host's log table are
 * containers, and most of them are on nobody's dashboard — waiting for one to
 * be placed before its log could be read would make the table a list of things
 * you mostly cannot open.
 *
 * A log row gets the one log binding it explicitly asks to open. Other
 * navigable resource rows (TrueNAS pools and datasets) get no invented widget
 * layout: their target-scoped resource kinds and actions still appear, while
 * guessing a connector-specific dashboard layout here would create a second,
 * worse copy of `default_layout_for` in shared UI.
 */
function targetPlacement(
  host: DashboardPlacement,
  targetId: string,
  includeLog: boolean,
): DashboardPlacement {
  return {
    ...host,
    // A distinct id so React does not reconcile the nested modal's subtree with
    // the host's, and so any keyed cache below it stays separate.
    id: `${host.id}:${targetId}`,
    targetId,
    widgetBindings: includeLog
      ? [{ display: { dataPointId: LOG_DATA_POINT_ID, widgetType: "logStream", config: {} } }]
      : [],
  };
}

/**
 * The data point a container's log lives under.
 *
 * A connector-level id in shared UI, which is the kind of thing this codebase
 * otherwise avoids — but the same one `logs` resource kind is already the
 * documented special case here, and the binding has to name *something*. It is
 * `logs` for every connector that publishes a log today, and a connector that
 * used another id would simply render an unresolved binding rather than
 * anything broken.
 */
const LOG_DATA_POINT_ID = "logs";

function ConnectorDetailSkeleton() {
  return <div className="space-y-5"><div className="grid gap-3 sm:grid-cols-2"><Skeleton className="h-8" /><Skeleton className="h-8" /></div><Skeleton className="h-48 w-full" /><Skeleton className="h-24 w-full" /></div>;
}

function formatChecked(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
}

/** Charts and logs need horizontal room and are much taller than scalar
 * widgets. Giving them their own row prevents a short StatTile beside one from
 * leaving what looks like a large missing-content hole in the expanded grid. */
function expandedWidgetSpansRow(binding: DashboardPlacement["widgetBindings"][number]): boolean {
  return "display" in binding &&
    (typeof binding.display.widgetType !== "string" || binding.display.widgetType === "logStream");
}
