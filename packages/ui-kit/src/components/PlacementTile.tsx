import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  FolderInput,
  GripVertical,
  Maximize2,
  Pencil,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { ConnectorDetailModal } from "@loom/ui-kit/components/ConnectorDetailModal";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent, CardHeader } from "@loom/ui-kit/components/ui/card";
import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorError,
  type ConnectorStatus,
  type DashboardPlacement,
  type PendingOperation,
} from "@loom/ui-kit/lib/api";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { cn } from "@loom/ui-kit/lib/utils";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";
import { useRetainedStatusDetails } from "@loom/ui-kit/lib/use-retained-status-details";
import { renderWidget } from "@loom/ui-kit/widgets/renderWidget";

/** The live reading for one instance, as pushed over the status socket. */
export type LiveStatus = {
  status: ConnectorStatus | null;
  statusError?: ConnectorError;
  /** A disruptive action in flight, which outranks health on screen. */
  pendingOperation?: PendingOperation | null;
  /** Why this instance is Down, probed from the network beneath it. */
  diagnosis?: string | null;
};

/** The class the grid is told to treat as the drag handle. Only the header
 *  carries it, so a drag can never start on a slider or a button. */
export const DRAG_HANDLE_CLASS = "loom-drag-handle";

/**
 * One placed connector: its card shell and every widget bound to it.
 *
 * ## Where its data comes from
 *
 * Three sources, and keeping them apart is the point:
 *
 * - the **placement** supplies the bindings and the geometry, from dashboard
 *   detail;
 * - the **instance detail** supplies `dataPoints` and `actions` — the labels,
 *   units and parameter schemas a binding resolves against. Fetched here under
 *   the same `["connector-instance", id]` key `ConnectorCard` uses, so two
 *   placements of one connector share a single request;
 * - the **live status** supplies the values, arriving on the WebSocket and
 *   passed down from `DashboardView` rather than subscribed to per card. One
 *   subscription for the whole dashboard is what the socket's reference
 *   counting is for.
 *
 * Only the values change on a poll, which is why they are the only ones on the
 * push channel. A card whose detail is still loading renders its header and
 * skeletons its widget area rather than skeletoning the whole card — the name
 * and health are already known, and hiding them would look like a reload.
 */
export function PlacementTile({
  placement,
  live,
  editing,
  onEditBindings,
  onDelete,
  grouping = false,
  selected = false,
  onSelectedChange,
  onAddToGroup,
  groupMember,
}: {
  placement: DashboardPlacement;
  /** The latest pushed reading, or `undefined` before the first frame — in
   *  which case the placement's own snapshot is used. */
  live?: LiveStatus;
  /** Whether the dashboard is in layout-edit mode. Owner/Editor only. */
  editing: boolean;
  onEditBindings: (placement: DashboardPlacement) => void;
  onDelete?: (placement: DashboardPlacement) => void;
  /** Selection mode for combining standalone tiles. */
  grouping?: boolean;
  selected?: boolean;
  onSelectedChange?: (selected: boolean) => void;
  /** Present only when at least one existing group can accept this tile. */
  onAddToGroup?: (placement: DashboardPlacement) => void;
  /** Member-only controls. A grouped tile is never a nested grid drag handle. */
  groupMember?: {
    index: number;
    total: number;
    pending: boolean;
    onMoveLeft: () => void;
    onMoveRight: () => void;
    onRemove: () => void;
  };
}) {
  const api = useApiClient();
  const { user } = useAuth();
  const instance = placement.connector;
  const [detailOpen, setDetailOpen] = React.useState(false);

  // Visibility only. **Not a security boundary**: the backend checks
  // `connectors.control` on every action request, scoped to this instance id,
  // and a dashboard share never grants it — see
  // docs/adr/0013-dashboard-sharing-model.md. A viewer who can see an action
  // widget still gets a 403 when they press it; disabling it here just stops
  // the interface from promising something it cannot deliver.
  const canControl = hasPermission(user?.permissions ?? [], PERMISSION_KEYS.connectorsControl);

  const detail = useQuery({
    queryKey: ["connector-instance", instance.id],
    queryFn: ({ signal }) => api.getConnectorInstance(instance.id, signal),
    retry: (failureCount, error) =>
      !(error instanceof ApiError && (error.isForbidden || error.status === 404)) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 1,
  });

  const execute = useMutation({
    mutationFn: ({ actionId, params }: { actionId: string; params: Record<string, unknown> }) =>
      api.executeConnectorAction(instance.id, actionId, params),
    onSettled: () => {
      // The action list can shift with the service's state. Status itself
      // arrives on the socket, so nothing re-reads it here.
      void detail.refetch();
    },
  });

  const runAction = React.useCallback(
    async (actionId: string, params: Record<string, unknown>) => {
      const label =
        detail.data?.actions.find((action) => action.id === actionId)?.label ?? actionId;
      try {
        const result = await execute.mutateAsync({ actionId, params });
        // A 200 with `success: false` means the service was reached and
        // declined — a different thing from the request failing.
        if (result.success) {
          toast.success(`${instance.name}: ${label}`, { description: result.message });
        } else {
          toast.warning(`${instance.name}: ${label} declined`, { description: result.message });
        }
        return result;
      } catch (error) {
        toast.error(`${instance.name}: ${label} failed`, {
          description: describeConnectorError(error),
        });
        // Rethrown on purpose: the optimistic widgets roll themselves back on
        // a rejection, and swallowing it here would leave a toggle showing a
        // state the service never reached.
        throw error;
      }
    },
    [detail.data, execute, instance.name],
  );

  const status = live?.status ?? instance.status;
  const statusError = live === undefined ? instance.statusError : live.statusError;
  // One helper decides what the badge says and whether the controls work, so
  // "a pending operation outranks health" is a rule that exists once rather
  // than being re-derived in the tile, the modal and the dispatcher.
  const availability = connectorAvailability({
    status,
    statusError,
    pendingOperation: live === undefined ? instance.pendingOperation : live.pendingOperation,
    diagnosis: live === undefined ? instance.diagnosis : live.diagnosis,
  });
  const currentDetails =
    typeof status?.details === "object" && status.details !== null && !Array.isArray(status.details)
      ? (status.details as Record<string, unknown>)
      : {};
  const details = useRetainedStatusDetails(instance.id, currentDetails);

  if (detail.isPending) {
    return <PlacementTileSkeleton placement={placement} />;
  }

  return (
    <>
      <Card className="flex h-full flex-col overflow-hidden">
        <CardHeader
          className={cn(
            "flex-row items-center justify-between space-y-0 gap-2 py-3",
            groupMember !== undefined && "flex-wrap",
            // The whole header is the drag surface in edit mode, so a card can be
            // moved without aiming at a grip the size of a full stop.
            editing &&
              !grouping &&
              groupMember === undefined &&
              `${DRAG_HANDLE_CLASS} cursor-grab active:cursor-grabbing`,
          )}
        >
        <div className="flex min-w-0 items-center gap-2">
          {editing && !grouping && groupMember === undefined ? (
            <GripVertical
              className="h-4 w-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
          ) : null}
          <ConnectorIcon
            typeIcon={instance.metadata.icon}
            iconOverride={instance.iconOverride}
            size={20}
          />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold leading-none" title={instance.name}>
              {instance.name}
            </p>
            <p className="truncate text-xs text-muted-foreground">{instance.metadata.name}</p>
          </div>
        </div>

        <div
          className={cn(
            "flex shrink-0 items-center gap-1",
            groupMember !== undefined && "w-full justify-end border-t pt-1",
          )}
        >
          {grouping ? (
            <Checkbox
              className="loom-grid-control size-8 rounded-md"
              checked={selected}
              aria-label={`${selected ? "Deselect" : "Select"} ${instance.name} for grouping`}
              onCheckedChange={(checked) => onSelectedChange?.(checked === true)}
            />
          ) : null}
          <Badge variant={availability.tone} title={availability.label}>
            {availability.label}
          </Badge>
          {grouping ? null : editing ? (
            <>
              {groupMember !== undefined ? (
                <>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="loom-grid-control h-7 w-7"
                    disabled={groupMember.pending || groupMember.index === 0}
                    aria-label={`Move ${instance.name} left`}
                    onClick={groupMember.onMoveLeft}
                  >
                    <ArrowLeft aria-hidden="true" />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="loom-grid-control h-7 w-7"
                    disabled={groupMember.pending || groupMember.index === groupMember.total - 1}
                    aria-label={`Move ${instance.name} right`}
                    onClick={groupMember.onMoveRight}
                  >
                    <ArrowRight aria-hidden="true" />
                  </Button>
                </>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="loom-grid-control h-7 w-7"
                aria-label={`Edit widgets on ${instance.name}`}
                onClick={() => onEditBindings(placement)}
              >
                <Pencil aria-hidden="true" />
              </Button>
              {groupMember !== undefined ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="loom-grid-control h-7 w-7 text-muted-foreground hover:text-destructive"
                  disabled={groupMember.pending}
                  aria-label={`Remove ${instance.name} from group`}
                  onClick={groupMember.onRemove}
                >
                  <X aria-hidden="true" />
                </Button>
              ) : (
                <>
                  {onAddToGroup !== undefined ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="loom-grid-control h-7 w-7"
                      aria-label={`Add ${instance.name} to a group`}
                      onClick={() => onAddToGroup(placement)}
                    >
                      <FolderInput aria-hidden="true" />
                    </Button>
                  ) : null}
                  {onDelete !== undefined ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="loom-grid-control h-7 w-7 text-muted-foreground hover:text-destructive"
                      aria-label={`Remove ${instance.name} from this dashboard`}
                      onClick={() => onDelete(placement)}
                    >
                      <Trash2 aria-hidden="true" />
                    </Button>
                  ) : null}
                </>
              )}
            </>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="loom-grid-control h-7 w-7"
              aria-label={`Expand ${instance.name}`}
              onClick={() => setDetailOpen(true)}
              title={`Open ${instance.name} details`}
            >
              <Maximize2 aria-hidden="true" />
            </Button>
          )}
        </div>
        </CardHeader>

        <CardContent className="min-h-0 flex-1 overflow-auto pb-4">
        {statusError !== undefined ? (
          <Alert variant="destructive" className="mb-3">
            <AlertCircle aria-hidden="true" />
            <AlertDescription>{describeConnectorError(statusError)}</AlertDescription>
          </Alert>
        ) : null}

        {/* The network-level explanation, when there is one. A plain line
            rather than another Alert: it sits under a Badge that has already
            said something is wrong, and a second red box would be shouting the
            same thing twice. */}
        {availability.diagnosis !== null ? (
          <p className="mb-3 text-xs leading-relaxed text-muted-foreground">
            {availability.diagnosis}
          </p>
        ) : null}

        {detail.isError ? (
          <Alert variant="destructive">
            <AlertCircle aria-hidden="true" />
            <AlertDescription>{describeConnectorError(detail.error)}</AlertDescription>
          </Alert>
        ) : placement.widgetBindings.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No widgets are bound to this placement yet.
          </p>
        ) : (
          <div className="grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(9rem,1fr))]">
            {placement.widgetBindings.map((binding, index) => (
              <React.Fragment key={index}>
                {renderWidget({
                  binding,
                  statusDetails: details,
                  dataPoints: detail.data.dataPoints,
                  actions: detail.data.actions,
                  onExecute: runAction,
                  // Controls are dead while the layout is being rearranged: a
                  // click meant to grab a card should not restart a service.
                  disabled: !canControl || editing,
                  // ...and dead, *with an explanation*, when the connector
                  // cannot be reached at all. A button that fails on click
                  // teaches nothing; one that says why before the click does.
                  unavailableReason: availability.unavailableReason,
                  className:
                    // A chart or a log pane needs room; the scalar widgets do
                    // not. Spanning them is what keeps a mixed card readable
                    // without the binding having to carry a size.
                    "display" in binding &&
                    (typeof binding.display.widgetType !== "string" ||
                      binding.display.widgetType === "logStream")
                      ? "col-span-full min-h-[8rem]"
                      : undefined,
                })}
              </React.Fragment>
            ))}
          </div>
        )}
        </CardContent>
      </Card>
      <ConnectorDetailModal
        placement={placement}
        open={detailOpen}
        onOpenChange={setDetailOpen}
      />
    </>
  );
}

function PlacementTileSkeleton({ placement }: { placement: DashboardPlacement }) {
  return (
    <Card className="flex h-full flex-col overflow-hidden" aria-label="Loading connector placement">
      <CardHeader className="flex-row items-center justify-between space-y-0 gap-2 py-3">
        <div className="flex flex-col gap-2">
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-3 w-20" />
        </div>
        <Skeleton className="h-6 w-16 rounded-full" />
      </CardHeader>
      <CardContent className="grid flex-1 gap-4 [grid-template-columns:repeat(auto-fit,minmax(9rem,1fr))]">
        {Array.from({ length: Math.max(1, placement.widgetBindings.length) }, (_, index) => (
          <Skeleton key={index} className="h-20 w-full" />
        ))}
      </CardContent>
    </Card>
  );
}
