import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ResponsiveGridLayout,
  useContainerWidth,
  type Layout,
  type LayoutItem,
  type ResponsiveLayouts,
} from "react-grid-layout";
import { AlertCircle, Check, LayoutGrid, Pencil, Plus, Share2, Trash2 } from "lucide-react";
import { toast } from "sonner";

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
import { Card, CardContent } from "@loom/ui-kit/components/ui/card";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { AddPlacementDialog } from "@loom/ui-kit/components/AddPlacementDialog";
import { DashboardSharesDialog } from "@loom/ui-kit/components/DashboardSharesDialog";
import { dashboardsQueryKey } from "@loom/ui-kit/components/DashboardSidebar";
import {
  DashboardPlacementCard,
  DRAG_HANDLE_CLASS,
  type LiveStatus,
} from "@loom/ui-kit/components/DashboardPlacementCard";
import { PlacementBindingsDialog } from "@loom/ui-kit/components/PlacementBindingsDialog";
import type {
  DashboardDetail,
  DashboardPlacement,
  DashboardSummary,
} from "@loom/ui-kit/lib/api";
import { useApiClient, useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

const dashboardQueryKey = (dashboardId: string) => ["dashboard", dashboardId] as const;

/**
 * Grid geometry.
 *
 * The column count is chosen against what `metadata.minSize` claims to mean:
 * "the smallest footprint at which this connector is still readable". A
 * twelve-column grid makes the common declaration of `[2, 2]` one sixth of the
 * page — far too narrow to read anything — which would quietly turn every
 * connector's minimum into a lie. Six columns puts that same declaration at a
 * third of the width, which is a real card, and still divides evenly by two and
 * three for halves and thirds.
 *
 * The row height follows the same logic from the other side: two rows plus the
 * margin between them is about the height of a card header and a couple of
 * widgets.
 */
const GRID_COLS = 6;
const GRID_ROW_HEIGHT = 110;
const GRID_MARGIN: [number, number] = [16, 16];
const GRID_BREAKPOINTS = { lg: 768, md: 600, sm: 420, xs: 0 } as const;
const GRID_COLUMNS = { lg: 6, md: 4, sm: 2, xs: 1 } as const;
type GridBreakpoint = keyof typeof GRID_BREAKPOINTS;

function layoutFromPlacements(placements: DashboardPlacement[]): Layout {
  return placements.map((placement) => ({
    i: placement.id,
    x: placement.positionX,
    y: placement.positionY,
    w: placement.width,
    h: placement.height,
    minW: placement.connector.metadata.minSize[0],
    minH: placement.connector.metadata.minSize[1],
  }));
}

function scaledLayout(placements: DashboardPlacement[], columns: number): Layout {
  return placements.map((placement) => {
    const minimumWidth = Math.min(placement.connector.metadata.minSize[0], columns);
    const width = Math.min(
      columns,
      Math.max(minimumWidth, Math.ceil((placement.width * columns) / GRID_COLS)),
    );
    return {
      i: placement.id,
      x: Math.min(Math.floor((placement.positionX * columns) / GRID_COLS), columns - width),
      y: placement.positionY,
      w: width,
      h: placement.height,
      minW: minimumWidth,
      minH: placement.connector.metadata.minSize[1],
    };
  });
}

function stackedLayout(placements: DashboardPlacement[], columns: number): Layout {
  let nextRow = 0;
  return [...placements]
    .sort(
      (left, right) =>
        left.positionY - right.positionY || left.positionX - right.positionX,
    )
    .map((placement) => {
      const item = {
        i: placement.id,
        x: 0,
        y: nextRow,
        w: columns,
        h: placement.height,
        minW: columns,
        minH: placement.connector.metadata.minSize[1],
      };
      nextRow += placement.height;
      return item;
    });
}

function responsiveLayoutsFromPlacements(
  placements: DashboardPlacement[],
): ResponsiveLayouts<GridBreakpoint> {
  return {
    lg: layoutFromPlacements(placements),
    md: scaledLayout(placements, GRID_COLUMNS.md),
    sm: stackedLayout(placements, GRID_COLUMNS.sm),
    xs: stackedLayout(placements, GRID_COLUMNS.xs),
  };
}

function breakpointForWidth(width: number): GridBreakpoint {
  if (width >= GRID_BREAKPOINTS.lg) return "lg";
  if (width >= GRID_BREAKPOINTS.md) return "md";
  if (width >= GRID_BREAKPOINTS.sm) return "sm";
  return "xs";
}

/**
 * One dashboard: its header, and its placements as a drag-and-drop grid.
 *
 * ## Read-only by default
 *
 * The grid is static until an Owner or Editor turns on **Edit layout**, and a
 * Viewer never sees that button. This is not only about permissions: a
 * dashboard is something people look at far more often than they rearrange, and
 * a grid that is always draggable turns every attempt to click a button into a
 * chance to move a card. Edit mode also disables the action widgets themselves,
 * so a grab that lands on a toggle cannot restart a service.
 *
 * ## Where live values come from
 *
 * One WebSocket subscription for every instance on the dashboard, held here
 * rather than in each card. The socket reference-counts its subscriptions, so
 * two placements of the same connector cost one subscription, and the readings
 * are handed down as props. Only `status` travels this way — the labels, units
 * and action schemas a binding resolves against come from each card's own
 * instance-detail query, because those change when a connector is reconfigured
 * and not on every poll.
 *
 * ## What gets persisted, and when
 *
 * Only on drag-stop and resize-stop, never per frame. The grid's compactor can
 * move cards the user did not touch, so the whole layout is compared against
 * what is stored and every changed placement is PATCHed — persisting just the
 * dragged card would leave the others showing a position the server does not
 * have. Local layout state updates immediately so nothing snaps back while the
 * requests are in flight.
 */
export function DashboardView({
  dashboardId,
  onDeleted,
}: {
  dashboardId: string;
  onDeleted: () => void;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const connectorSocket = useConnectorStatusSocket();
  const [editingName, setEditingName] = React.useState(false);
  const [name, setName] = React.useState("");
  const [sharesOpen, setSharesOpen] = React.useState(false);
  const [deleteOpen, setDeleteOpen] = React.useState(false);
  const [addOpen, setAddOpen] = React.useState(false);
  const [editingLayout, setEditingLayout] = React.useState(false);
  const [bindingsFor, setBindingsFor] = React.useState<DashboardPlacement | null>(null);
  const [removing, setRemoving] = React.useState<DashboardPlacement | null>(null);
  const [live, setLive] = React.useState<Record<string, LiveStatus>>({});
  const { width, containerRef, mounted, measureWidth } = useContainerWidth({
    measureBeforeMount: true,
  });

  const dashboard = useQuery({
    queryKey: dashboardQueryKey(dashboardId),
    queryFn: ({ signal }) => api.getDashboard(dashboardId, signal),
  });

  // Standalone placements only. `dashboard.data.placementGroups` — several
  // placements combined into one wider tile — is **not rendered yet**: the
  // backend and the API contract for grouping exist, the grid rendering for it
  // does not, and nothing in this app can create a group in the meantime. When
  // it lands it belongs here, as a second kind of grid item beside these, not
  // as a flattening of groups back into loose placements.
  const placements = React.useMemo(
    () => dashboard.data?.placements ?? [],
    [dashboard.data],
  );

  const responsiveLayouts = React.useMemo(
    () => responsiveLayoutsFromPlacements(placements),
    [placements],
  );

  const instanceIds = React.useMemo(
    () => [...new Set(placements.map((placement) => placement.connector.id))],
    [placements],
  );

  React.useEffect(() => {
    if (dashboard.data === undefined || placements.length === 0) return;
    // The first render returns DashboardViewSkeleton before the measured grid
    // container exists. useContainerWidth's mount effect therefore observes a
    // null ref and cannot mark itself mounted. Measure again once the dashboard
    // response has caused that container to enter the DOM; without this, a
    // cold direct navigation stays on the grid skeleton until the view is
    // unmounted and revisited with dashboard detail already in React Query.
    measureWidth();
  }, [dashboard.data, measureWidth, placements.length]);

  React.useEffect(() => {
    if (instanceIds.length === 0) return;
    return connectorSocket.subscribe(instanceIds, (update) => {
      setLive((current) => ({
        ...current,
        [update.instanceId]: { status: update.status, statusError: update.statusError },
      }));
    });
  }, [connectorSocket, instanceIds]);

  React.useEffect(() => {
    if (!editingName && dashboard.data !== undefined) setName(dashboard.data.name);
  }, [dashboard.data, editingName]);

  const refreshDashboard = React.useCallback(
    () => queryClient.invalidateQueries({ queryKey: dashboardQueryKey(dashboardId) }),
    [dashboardId, queryClient],
  );

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
  const removePlacement = useMutation({
    mutationFn: (placement: DashboardPlacement) =>
      api.deleteDashboardPlacement(dashboardId, placement.id),
    onSuccess: async () => {
      await refreshDashboard();
      setRemoving(null);
    },
  });

  const persist = React.useCallback(
    async (next: Layout, breakpoint: GridBreakpoint) => {
      const stored = new Map(placements.map((placement) => [placement.id, placement]));
      // The API stores one canonical layout. At narrower breakpoints the grid
      // is derived from it, so preserve desktop x/width while still allowing
      // touch users to adjust vertical order and height.
      const canonical = next.map((item) => {
        const placement = stored.get(item.i);
        return breakpoint === "lg" || placement === undefined
          ? item
          : { ...item, x: placement.positionX, w: placement.width };
      });
      const moved = canonical.filter((item) => {
        const placement = stored.get(item.i);
        return (
          placement !== undefined &&
          (placement.positionX !== item.x ||
            placement.positionY !== item.y ||
            placement.width !== item.w ||
            placement.height !== item.h)
        );
      });
      if (moved.length === 0) return;

      try {
        await Promise.all(
          moved.map((item) =>
            api.updateDashboardPlacement(dashboardId, item.i, {
              positionX: item.x,
              positionY: item.y,
              width: item.w,
              height: item.h,
            }),
          ),
        );
      } catch (error) {
        toast.error("Could not save the layout", {
          description: describeConnectorError(error),
        });
      } finally {
        // Refetched either way: on success to confirm, and on failure to snap
        // the grid back to what the server actually holds rather than leaving
        // it showing a position that was rejected.
        await refreshDashboard();
      }
    },
    [api, dashboardId, placements, refreshDashboard],
  );

  const onLayoutSettled = React.useCallback(
    (next: Layout, _old: LayoutItem | null) => {
      void persist(next, breakpointForWidth(width));
    },
    [persist, width],
  );

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

  const detail: DashboardDetail = dashboard.data;
  const isOwner = detail.role === "owner";
  const canEdit = isOwner || detail.role === "editor";

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

        <div className="flex flex-wrap items-center justify-end gap-2">
          {canEdit ? (
            <>
              <Button
                type="button"
                variant={editingLayout ? "default" : "outline"}
                size="sm"
                aria-pressed={editingLayout}
                onClick={() => setEditingLayout((current) => !current)}
              >
                {editingLayout ? <Check aria-hidden="true" /> : <LayoutGrid aria-hidden="true" />}
                {editingLayout ? "Done" : "Edit layout"}
              </Button>
              <Button type="button" variant="outline" size="sm" onClick={() => setAddOpen(true)}>
                <Plus aria-hidden="true" />
                Add connector
              </Button>
            </>
          ) : null}
          {isOwner ? (
            <>
              <Button type="button" variant="outline" size="sm" onClick={() => setSharesOpen(true)}>
                <Share2 aria-hidden="true" />
                Share
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                onClick={() => setDeleteOpen(true)}
              >
                <Trash2 aria-hidden="true" />
                Delete
              </Button>
            </>
          ) : null}
        </div>
      </div>

      {editingLayout ? (
        <Alert>
          <LayoutGrid aria-hidden="true" />
          <AlertTitle>Editing the layout</AlertTitle>
          <AlertDescription>
            Drag a card by its header to move it, or its bottom-right corner to resize it. Widget
            controls are disabled while you rearrange.
          </AlertDescription>
        </Alert>
      ) : null}

      {placements.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
            <p className="font-medium">No connectors placed on this dashboard yet.</p>
            <p className="max-w-md text-sm text-muted-foreground">
              {canEdit
                ? "Add a connector to start building this dashboard. You choose which of its readings and controls appear."
                : "Nothing has been placed here yet. Ask the dashboard's owner to add a connector."}
            </p>
            {canEdit ? (
              <Button type="button" onClick={() => setAddOpen(true)}>
                <Plus aria-hidden="true" />
                Add connector
              </Button>
            ) : null}
          </CardContent>
        </Card>
      ) : (
        <div
          // react-grid-layout 2.x is typed against React 19, where a ref object
          // is `RefObject<T | null>`; React 18's `ref` prop wants
          // `RefObject<T>`. The runtime value is identical — React assigns
          // `.current` either way — so this narrows the types-only difference
          // rather than asserting anything about the value. Remove when the
          // clients move to React 19.
          ref={containerRef as React.RefObject<HTMLDivElement>}
          className="min-w-0"
        >
          {mounted ? (
            <ResponsiveGridLayout<GridBreakpoint>
              width={width}
              breakpoints={GRID_BREAKPOINTS}
              cols={GRID_COLUMNS}
              layouts={responsiveLayouts}
              rowHeight={GRID_ROW_HEIGHT}
              margin={{ lg: GRID_MARGIN, md: GRID_MARGIN, sm: [12, 12], xs: [12, 12] }}
              // Gates the resize grip in CSS. The library keeps rendering the
              // handle element even with resizing disabled, and a grip that
              // appears on hover and then refuses to move is worse than no grip
              // — particularly for a Viewer, who has no way to make it work.
              className={editingLayout ? "loom-grid-editing" : undefined}
              // The header is the only drag surface, so a press on a slider or a
              // button inside a card never becomes a drag.
              dragConfig={{ enabled: editingLayout, handle: `.${DRAG_HANDLE_CLASS}` }}
              resizeConfig={{ enabled: editingLayout }}
              onDragStop={onLayoutSettled}
              onResizeStop={onLayoutSettled}
            >
              {placements.map((placement) => (
                <div key={placement.id} className="min-w-0">
                  <DashboardPlacementCard
                    placement={placement}
                    live={live[placement.connector.id]}
                    editing={editingLayout}
                    onEditBindings={setBindingsFor}
                    onDelete={setRemoving}
                  />
                </div>
              ))}
            </ResponsiveGridLayout>
          ) : (
            <Skeleton className="h-48 w-full" />
          )}
        </div>
      )}

      {canEdit ? (
        <>
          <AddPlacementDialog
            dashboardId={detail.id}
            existingPlacements={placements}
            open={addOpen}
            onOpenChange={setAddOpen}
            onCreated={refreshDashboard}
          />
          <PlacementBindingsDialog
            dashboardId={detail.id}
            placement={bindingsFor}
            onOpenChange={(open) => {
              if (!open) setBindingsFor(null);
            }}
            onSaved={refreshDashboard}
          />
        </>
      ) : null}

      {isOwner ? (
        <DashboardSharesDialog
          dashboardId={detail.id}
          dashboardName={detail.name}
          open={sharesOpen}
          onOpenChange={setSharesOpen}
        />
      ) : null}

      <AlertDialog
        open={removing !== null}
        onOpenChange={(next) => {
          if (!next) {
            setRemoving(null);
            removePlacement.reset();
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {removing?.connector.name} from this dashboard?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes the card and its widgets. The connector itself, and any other dashboard
              it appears on, are untouched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {removePlacement.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>
                {describeConnectorError(removePlacement.error)}
              </AlertDescription>
            </Alert>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removePlacement.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={removePlacement.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (removing !== null) removePlacement.mutate(removing);
              }}
            >
              {removePlacement.isPending ? "Removing…" : "Remove"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

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
