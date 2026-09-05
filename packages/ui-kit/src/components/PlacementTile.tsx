import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  FolderInput,
  GripVertical,
  Layers,
  Loader2,
  Maximize2,
  Pencil,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { ConnectorDetailModal } from "@loom/ui-kit/components/ConnectorDetailModal";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent, CardHeader } from "@loom/ui-kit/components/ui/card";
import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorError,
  type ConnectorInstanceSummary,
  type ConnectorStatus,
  type DashboardPlacement,
  type PendingOperation,
} from "@loom/ui-kit/lib/api";
import { PlacementClickSurface, usePlacementClick } from "@loom/ui-kit/components/PlacementClick";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { ConnectorStatusBadge } from "@loom/ui-kit/components/ConnectorStatusBadge";
import { cn } from "@loom/ui-kit/lib/utils";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { describeTarget } from "@loom/ui-kit/lib/target-label";
import {
  matchesTarget,
  statusDetailsForTarget,
} from "@loom/ui-kit/lib/connector-details";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";
import { useRetainedStatusDetails } from "@loom/ui-kit/lib/use-retained-status-details";
import { renderWidget } from "@loom/ui-kit/widgets/renderWidget";
import { UpdatesSummary, UPDATES_KIND } from "@loom/ui-kit/components/UpdatesSummary";

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
  dashboardId,
  placement,
  live,
  editing,
  onEditBindings,
  onDelete,
  onNavigateDashboard,
  grouping = false,
  selected = false,
  onSelectedChange,
  onAddToGroup,
  groupMember,
}: {
  /** The dashboard this tile lives on. The click endpoint is scoped to it. */
  dashboardId: string;
  placement: DashboardPlacement;
  /** The latest pushed reading, or `undefined` before the first frame — in
   *  which case the placement's own snapshot is used. */
  live?: LiveStatus;
  /** Whether the dashboard is in layout-edit mode. Owner/Editor only. */
  editing: boolean;
  onEditBindings: (placement: DashboardPlacement) => void;
  onDelete?: (placement: DashboardPlacement) => void;
  /**
   * Opens another dashboard, for a tile whose click navigates.
   *
   * Supplied by the host rather than done here: the UI kit ships to three
   * clients and knows nothing about any of their routers. Omitted where there
   * is nowhere to go, and a navigate tile is then simply not clickable rather
   * than clickable and inert.
   */
  onNavigateDashboard?: (dashboardId: string) => void;
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
  const props = {
    dashboardId,
    placement,
    live,
    editing,
    onEditBindings,
    onDelete,
    onNavigateDashboard,
    grouping,
    selected,
    onSelectedChange,
    onAddToGroup,
    groupMember,
  };

  // A placement with no connector has no status, no instance detail to fetch
  // and nothing to bind, so it is a different card rather than this one with
  // every section suppressed. Two components rather than one full of `?.`:
  // the connector card's whole body — status, availability, widgets, the
  // detail modal — is meaningless without an instance, and a branch here is
  // cheaper to read than a branch at every use of one.
  //
  // Dispatch happens before any hook, and neither branch is conditional inside
  // itself, so the rules of hooks hold on both paths.
  return placement.connector === null ? (
    <StaticPlacementTile {...props} />
  ) : (
    <ConnectorPlacementTile {...props} connector={placement.connector} />
  );
}

/** Props shared by both tile bodies; see `PlacementTile` for what each means. */
type PlacementTileProps = {
  dashboardId: string;
  placement: DashboardPlacement;
  live?: LiveStatus;
  editing: boolean;
  onEditBindings: (placement: DashboardPlacement) => void;
  onDelete?: (placement: DashboardPlacement) => void;
  onNavigateDashboard?: (dashboardId: string) => void;
  grouping?: boolean;
  selected?: boolean;
  onSelectedChange?: (selected: boolean) => void;
  onAddToGroup?: (placement: DashboardPlacement) => void;
  groupMember?: {
    index: number;
    total: number;
    pending: boolean;
    onMoveLeft: () => void;
    onMoveRight: () => void;
    onRemove: () => void;
  };
};

/**
 * A tile that shows a connector: the card shell, and every widget bound to it.
 *
 * The original `PlacementTile` body, unchanged but for two additions — the
 * resource-kind bindings it now hands to `renderWidget`, and the click surface
 * wrapped around the whole card when the placement carries a `placementAction`.
 */
function ConnectorPlacementTile({
  dashboardId,
  placement,
  connector: instance,
  live,
  editing,
  onEditBindings,
  onDelete,
  onNavigateDashboard,
  grouping = false,
  selected = false,
  onSelectedChange,
  onAddToGroup,
  groupMember,
}: PlacementTileProps & { connector: ConnectorInstanceSummary }) {
  const api = useApiClient();
  const { user } = useAuth();
  const [detailOpen, setDetailOpen] = React.useState(false);
  // Which part of the detail view was asked for. The header's expand button
  // means "show me this connector"; a log preview's means "show me *that*",
  // and a modal that opens on a stat tile has ignored the question.
  const [detailFocus, setDetailFocus] = React.useState<"logs" | null>(null);
  const openDetail = React.useCallback((focus: "logs" | null = null) => {
    setDetailFocus(focus);
    setDetailOpen(true);
  }, []);

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
  const subTargets = useQuery({
    queryKey: ["connector-instance-sub-targets", instance.id],
    queryFn: ({ signal }) => api.getSubTargets(instance.id, signal),
    enabled: placement.targetId !== null,
    staleTime: 30_000,
  });

  const execute = useMutation({
    mutationFn: ({ actionId, params }: { actionId: string; params: Record<string, unknown> }) =>
      api.executeConnectorAction(instance.id, actionId, params, placement.targetId),
    onSettled: () => {
      // The action list can shift with the service's state. Status itself
      // arrives on the socket, so nothing re-reads it here.
      void detail.refetch();
    },
  });

  const runAction = React.useCallback(
    async (actionId: string, params: Record<string, unknown>) => {
      const label =
        detail.data?.actions.find(
          (action) => matchesTarget(action, placement.targetId) && action.id === actionId,
        )?.label ?? actionId;
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
    [detail.data, execute, instance.name, placement.targetId],
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
  const targetHealth =
    placement.targetId === null ? undefined : status?.targetHealth?.[placement.targetId];
  // Per-target health changes only the header badge. Action availability must
  // continue to follow the connector-level transport state: a stopped Docker
  // container is correctly `down`, but its Start action must remain usable.
  const badgeAvailability =
    targetHealth === undefined || status === null
      ? availability
      : connectorAvailability({
          status: { ...status, health: targetHealth },
          statusError,
          pendingOperation:
            live === undefined ? instance.pendingOperation : live.pendingOperation,
          diagnosis: live === undefined ? instance.diagnosis : live.diagnosis,
        });
  // Docker's id remains a useful immediate fallback. The shared cached target
  // lookup adds authoritative labels and icons for connectors such as UniFi,
  // whose stable UUID cannot itself tell a person which device it names.
  const targetMetadata = subTargets.data?.find((target) => target.id === placement.targetId);
  const target = targetMetadata === undefined
    ? describeTarget(placement.targetId)
    : { text: targetMetadata.label, isStack: targetMetadata.kind === "stack" };
  const currentDetails = statusDetailsForTarget(status?.details, placement.targetId);
  const details = useRetainedStatusDetails(
    `${instance.id}:${placement.targetId ?? ""}`,
    currentDetails,
  );
  const targetDataPoints =
    detail.data?.dataPoints.filter((point) => matchesTarget(point, placement.targetId)) ?? [];
  const targetActions =
    detail.data?.actions.filter((action) => matchesTarget(action, placement.targetId)) ?? [];

  // Only for a host-level tile, and only when the connector says it has an
  // `updates` kind at all. A per-container tile browses nothing of its own, and
  // a connector without the kind must look exactly as it did before any of this
  // existed — no badge, no button, no request.
  // A `resourceKindDisplay` binding resolves against this same list, so the
  // query is no longer host-only — and its key now carries the target, which it
  // could get away with omitting while it only ever ran for the host view.
  const bindsResourceKind = placement.widgetBindings.some(
    (binding) => "resourceKindDisplay" in binding,
  );
  const resourceKinds = useQuery({
    queryKey: ["connector-resource-kinds", instance.id, placement.targetId],
    queryFn: ({ signal }) => api.getResourceKinds(instance.id, placement.targetId, signal),
    enabled: placement.targetId === null || bindsResourceKind,
    // Which kinds exist is a property of the connector, not of its state: it
    // changes when someone edits a configuration, not between polls.
    staleTime: 5 * 60_000,
  });
  const updatesKind =
    placement.targetId === null
      ? resourceKinds.data?.find((kind) => kind.kind === UPDATES_KIND)
      : undefined;

  const click = usePlacementClick({ dashboardId, placement, onNavigateDashboard });
  // Never while rearranging, for the same reason every action widget goes dead
  // there: a press meant to grab a card must not change the page underneath a
  // drag. Grouping-selection mode owns the click too.
  const clickable = click.clickable && !editing && !grouping;

  // A tile whose only binding is a resource kind *is* that table. Wrapping it
  // in the auto-fit widget grid would put a scrollable table inside a
  // fixed-width column inside a card, which is the awkward embedding this
  // binding exists to avoid.
  const isResourceKindTile =
    placement.widgetBindings.length === 1 && bindsResourceKind;

  if (detail.isPending) {
    return <PlacementTileSkeleton placement={placement} />;
  }

  return (
    <>
      <PlacementClickSurface
        active={clickable}
        pending={click.pending}
        label={clickLabel(placement, instance.name)}
        onActivate={click.run}
      >
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
          {/* A stack is not the connector's own thing — it is a set of them —
              so it takes the layers glyph rather than the Docker whale. A
              container placement keeps the icon it has always had. */}
          {targetMetadata?.icon !== undefined ? (
            <ConnectorIcon typeIcon={targetMetadata.icon} iconOverride={null} size={20} />
          ) : target?.isStack === true ? (
            <Layers className="shrink-0 text-muted-foreground" size={20} aria-hidden="true" />
          ) : (
            <ConnectorIcon
              typeIcon={instance.metadata.icon}
              iconOverride={instance.iconOverride}
              size={20}
            />
          )}
          <div className="min-w-0">
            <p className="whitespace-normal text-sm font-semibold leading-tight [overflow-wrap:anywhere]">
              {instance.name}
            </p>
            <p
              className="whitespace-normal text-xs text-muted-foreground [overflow-wrap:anywhere]"
            >
              {instance.metadata.name}
              {target === null ? null : ` · ${target.text}`}
            </p>
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
          <ConnectorStatusBadge availability={badgeAvailability} />
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
              // Named by target as well as instance: one connector can place
              // several tiles on one dashboard, and "Expand Docker host" three
              // times over is three controls a screen reader cannot tell apart.
              aria-label={
                target === null
                  ? `Expand ${instance.name}`
                  : `Expand ${instance.name} · ${target.text}`
              }
              onClick={() => openDetail()}
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

        {updatesKind === undefined ? null : (
          <UpdatesSummary
            instanceId={instance.id}
            descriptor={updatesKind}
            // Same rule as every other control on the tile: dead while the
            // layout is being rearranged, and dead with an explanation when the
            // connector cannot be reached.
            disabled={!canControl || editing || availability.actionsDisabled}
            disabledReason={
              canControl
                ? availability.unavailableReason
                : "You do not have permission to control this connector."
            }
          />
        )}

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
          <div
            className={cn(
              isResourceKindTile
                ? "flex h-full min-h-0 flex-col"
                : "grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(9rem,1fr))]",
            )}
          >
            {placement.widgetBindings.map((binding, index) => (
              <React.Fragment key={index}>
                {renderWidget({
                  binding,
                  statusDetails: details,
                  dataPoints: targetDataPoints,
                  actions: targetActions,
                  resourceKinds: resourceKinds.data,
                  instanceId: instance.id,
                  targetId: placement.targetId,
                  onExecute: runAction,
                  // Controls are dead while the layout is being rearranged: a
                  // click meant to grab a card should not restart a service.
                  disabled: !canControl || editing,
                  // ...and dead, *with an explanation*, when the connector
                  // cannot be reached at all. A button that fails on click
                  // teaches nothing; one that says why before the click does.
                  unavailableReason: availability.unavailableReason,
                  loading: status === null,
                  onExpand: () => openDetail("logs"),
                  className:
                    // A chart needs room; the scalar widgets do not. Spanning
                    // it is what keeps a mixed card readable without the
                    // binding having to carry a size. A log spans the row too —
                    // one line of monospace wants the width — but takes no
                    // height beyond it, because in the grid it *is* one line.
                    // A resource kind takes the whole footprint: it is the
                    // tile, not a widget sitting inside one.
                    "resourceKindDisplay" in binding
                      ? isResourceKindTile
                        ? "min-h-0 flex-1"
                        : "col-span-full min-h-[12rem]"
                      : "display" in binding && typeof binding.display.widgetType !== "string"
                        ? "col-span-full min-h-[8rem]"
                        : "display" in binding && binding.display.widgetType === "logStream"
                          ? "col-span-full"
                          : undefined,
                })}
              </React.Fragment>
            ))}
          </div>
        )}
        </CardContent>
      </Card>
      </PlacementClickSurface>
      <ConnectorDetailModal
        // Narrowed for the modal, which has no meaning without an instance.
        placement={{ ...placement, connector: instance }}
        open={detailOpen}
        onOpenChange={setDetailOpen}
        focus={detailFocus}
      />
    </>
  );
}

/**
 * A tile with no connector: an icon, a label, and a click.
 *
 * Deliberately austere. There is no status badge because there is no status,
 * no widget area because there is nothing bound, and no expand button because
 * there is no detail view to expand into — a shell that showed those things
 * empty would be reporting an outage the tile cannot have.
 *
 * The edit-mode affordances are the same ones a connector tile has, and for the
 * same reason: this is an ordinary placement, so it drags, resizes, groups and
 * deletes exactly like every other.
 */
function StaticPlacementTile({
  dashboardId,
  placement,
  editing,
  onEditBindings,
  onDelete,
  onNavigateDashboard,
  grouping = false,
  selected = false,
  onSelectedChange,
  onAddToGroup,
  groupMember,
}: PlacementTileProps) {
  const click = usePlacementClick({ dashboardId, placement, onNavigateDashboard });
  const clickable = click.clickable && !editing && !grouping;
  const label = placement.label ?? describePlacementAction(placement);

  return (
    <PlacementClickSurface
      active={clickable}
      pending={click.pending}
      label={clickLabel(placement, label)}
      onActivate={click.run}
    >
      <Card
        className={cn(
          "flex h-full flex-col overflow-hidden",
          // The whole card is the drag surface: there is no header to aim at,
          // and a one-by-one tile has no room for a grip.
          editing &&
            !grouping &&
            groupMember === undefined &&
            `${DRAG_HANDLE_CLASS} cursor-grab active:cursor-grabbing`,
        )}
      >
        {editing || grouping ? (
          <div className="flex shrink-0 items-center justify-end gap-1 px-2 pt-2">
            {grouping ? (
              <Checkbox
                className="loom-grid-control size-8 rounded-md"
                checked={selected}
                aria-label={`${selected ? "Deselect" : "Select"} ${label} for grouping`}
                onCheckedChange={(checked) => onSelectedChange?.(checked === true)}
              />
            ) : (
              <>
                {groupMember !== undefined ? (
                  <>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="loom-grid-control h-7 w-7"
                      disabled={groupMember.pending || groupMember.index === 0}
                      aria-label={`Move ${label} left`}
                      onClick={groupMember.onMoveLeft}
                    >
                      <ArrowLeft aria-hidden="true" />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="loom-grid-control h-7 w-7"
                      disabled={
                        groupMember.pending || groupMember.index === groupMember.total - 1
                      }
                      aria-label={`Move ${label} right`}
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
                  aria-label={`Edit ${label}`}
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
                    aria-label={`Remove ${label} from group`}
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
                        aria-label={`Add ${label} to a group`}
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
                        aria-label={`Remove ${label} from this dashboard`}
                        onClick={() => onDelete(placement)}
                      >
                        <Trash2 aria-hidden="true" />
                      </Button>
                    ) : null}
                  </>
                )}
              </>
            )}
          </div>
        ) : null}

        <CardContent className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-3 text-center">
          {click.pending ? (
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" aria-hidden="true" />
          ) : (
            <ConnectorIcon
              typeIcon={null}
              iconOverride={placement.icon}
              size={28}
              className={cn(
                "text-muted-foreground transition-colors",
                clickable && "group-hover:text-foreground",
              )}
            />
          )}
          <p className="whitespace-normal text-sm font-medium leading-tight [overflow-wrap:anywhere]">
            {label}
          </p>
        </CardContent>
      </Card>
    </PlacementClickSurface>
  );
}

/**
 * What a tile does, in words, when it has no label of its own.
 *
 * A fallback rather than a default: `label` is optional at the API, so a tile
 * created through a script may arrive without one, and an unlabelled square is
 * worse than a generic sentence.
 */
function describePlacementAction(placement: DashboardPlacement): string {
  const action = placement.placementAction;
  if (action === null) return "Button";
  return action.type === "navigate" ? "Open dashboard" : "Run action";
}

/** The accessible name of the whole-tile button. */
function clickLabel(placement: DashboardPlacement, fallback: string): string {
  const action = placement.placementAction;
  const name = placement.label ?? fallback;
  if (action?.type === "navigate") return `Open ${name}`;
  if (action?.type === "connectorAction") return `Run ${name}`;
  return name;
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
