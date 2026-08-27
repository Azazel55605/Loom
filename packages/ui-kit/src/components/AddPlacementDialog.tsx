import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { PlacementBindingEditor } from "@loom/ui-kit/components/PlacementBindingEditor";
import { SearchablePickerList } from "@loom/ui-kit/components/SearchablePickerList";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import type {
  DashboardPlacement,
  DashboardPlacementGroup,
  WidgetBinding,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

type PlacementMode = "server" | "target";

/**
 * The next free row, given what is already placed.
 *
 * Deliberately crude: everything new lands at the left edge, below the lowest
 * existing card. Anything cleverer would be guessing at an arrangement the user
 * is about to change anyway — the card is draggable the moment it appears, and
 * a placement that turned up in a surprising gap would be harder to find than
 * one that turned up at the bottom.
 */
function nextFreeRow(
  placements: DashboardPlacement[],
  placementGroups: DashboardPlacementGroup[],
): number {
  return Math.max(
    placements.reduce(
      (lowest, placement) => Math.max(lowest, placement.positionY + placement.height),
      0,
    ),
    placementGroups.reduce(
      (lowest, group) => Math.max(lowest, group.positionY + group.height),
      0,
    ),
  );
}

/**
 * Adds a connector to a dashboard.
 *
 * One dialog rather than a wizard. A host view reveals editable widgets,
 * pre-filled from the connector's own `defaultLayout`. A sub-target-capable
 * connector also offers one searchable target picker; target bindings are
 * seeded by the backend's target-aware default layout when the placement is
 * created.
 *
 * Size defaults to the connector's `metadata.minSize`, which is also the floor
 * the grid and the backend enforce, so a new card is never born too small to
 * read.
 */
export function AddPlacementDialog({
  dashboardId,
  existingPlacements,
  existingPlacementGroups = [],
  open,
  onOpenChange,
  onCreated,
}: {
  dashboardId: string;
  existingPlacements: DashboardPlacement[];
  existingPlacementGroups?: DashboardPlacementGroup[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void | Promise<void>;
}) {
  const api = useApiClient();
  const [instanceId, setInstanceId] = React.useState<string | null>(null);
  const [bindings, setBindings] = React.useState<WidgetBinding[]>([]);
  const [mode, setMode] = React.useState<PlacementMode>("server");
  const [targetId, setTargetId] = React.useState<string | null>(null);
  // Which instance's default layout has already been copied in, so re-renders
  // do not overwrite edits but choosing a different connector does.
  const seededFor = React.useRef<string | null>(null);

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: open,
  });

  const detail = useQuery({
    queryKey: ["connector-instance", instanceId],
    queryFn: ({ signal }) => api.getConnectorInstance(instanceId as string, signal),
    enabled: open && instanceId !== null,
  });

  const subTargets = useQuery({
    queryKey: ["connector-instance-sub-targets", instanceId],
    queryFn: ({ signal }) => api.getSubTargets(instanceId as string, signal),
    enabled:
      open &&
      instanceId !== null &&
      detail.data?.supportsSubTargets === true &&
      mode === "target",
  });

  React.useEffect(() => {
    if (detail.data === undefined) return;
    if (seededFor.current === detail.data.id) return;
    seededFor.current = detail.data.id;
    setMode("server");
    setTargetId(null);
    setBindings(detail.data.defaultLayout.bindings);
  }, [detail.data]);

  // Exactly one connector is the common case in a small deployment, and making
  // someone pick from a list of one is friction with no decision in it.
  React.useEffect(() => {
    if (!open || instanceId !== null) return;
    const only = instances.data?.length === 1 ? instances.data[0] : undefined;
    if (only !== undefined) setInstanceId(only.id);
  }, [instances.data, instanceId, open]);

  const create = useMutation({
    mutationFn: () => {
      const connector = detail.data;
      if (connector === undefined) throw new Error("no connector selected");
      const [width, height] = connector.metadata.minSize;
      return api.createDashboardPlacement(dashboardId, {
        connectorInstanceId: connector.id,
        targetId: mode === "target" ? targetId : null,
        positionX: 0,
        positionY: nextFreeRow(existingPlacements, existingPlacementGroups),
        width,
        height,
        // The detail endpoint publishes the host layout only. For a sub-target
        // the backend already applies `default_layout_for(targetId)` whenever
        // bindings are omitted, so it remains the one source of the initial
        // container layout rather than a second preview contract being added.
        ...(mode === "server" ? { widgetBindings: bindings } : {}),
      });
    },
    onSuccess: async () => {
      await onCreated();
      reset();
      onOpenChange(false);
    },
  });

  function reset() {
    setInstanceId(null);
    setBindings([]);
    setMode("server");
    setTargetId(null);
    seededFor.current = null;
    create.reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) reset();
      }}
    >
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add a connector</DialogTitle>
          <DialogDescription>
            Choose a connector instance and what the tile should show. You can rearrange and
            resize it afterwards.
          </DialogDescription>
        </DialogHeader>

        <form
          className="flex flex-col gap-5"
          // Native validation bubbles are a browser-default control per
          // docs/UI_GUIDELINES.md, and they would pre-empt the backend's own
          // message about a binding or a size it refused.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            if (instanceId !== null && (mode === "server" || targetId !== null)) {
              create.mutate();
            }
          }}
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="placement-instance">Connector</Label>
            {instances.isPending ? (
              <Skeleton className="h-9 w-full" />
            ) : instances.isError ? (
              <Alert variant="destructive">
                <AlertCircle aria-hidden="true" />
                <AlertDescription>{describeConnectorError(instances.error)}</AlertDescription>
              </Alert>
            ) : instances.data.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No connector instances exist yet. Add one under Connectors first.
              </p>
            ) : (
              <Select
                // Controlled from the first render: `undefined` would make
                // Radix treat it as uncontrolled, and the auto-select below
                // then flips it, which React warns about.
                value={instanceId ?? ""}
                disabled={create.isPending}
                onValueChange={(next) => {
                  setInstanceId(next);
                  setMode("server");
                  setTargetId(null);
                  setBindings([]);
                  seededFor.current = null;
                  create.reset();
                }}
              >
                <SelectTrigger id="placement-instance">
                  <SelectValue placeholder="Choose a connector" />
                </SelectTrigger>
                <SelectContent>
                  {instances.data.map((instance) => (
                    <SelectItem key={instance.id} value={instance.id}>
                      {instance.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>

          {instanceId === null ? null : detail.isPending ? (
            <Skeleton className="h-32 w-full" />
          ) : detail.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(detail.error)}</AlertDescription>
            </Alert>
          ) : (
            <div className="flex flex-col gap-4">
              {detail.data.supportsSubTargets ? (
                <div className="flex flex-col gap-2">
                  <Label>View</Label>
                  <SegmentedControl
                    label="Placement view"
                    value={mode}
                    options={[
                      { value: "server", label: "Server info" },
                      { value: "target", label: "Single container" },
                    ]}
                    onChange={(next) => {
                      setMode(next);
                      setTargetId(null);
                      create.reset();
                    }}
                  />
                </div>
              ) : null}

              {mode === "target" && detail.data.supportsSubTargets ? (
                <div className="flex min-h-0 flex-col gap-3">
                  <div>
                    <h3 className="text-sm font-medium">Container</h3>
                    <p className="text-xs text-muted-foreground">
                      Choose one container on {detail.data.metadata.name}. Its recommended
                      container widgets will be added automatically.
                    </p>
                  </div>
                  {subTargets.isPending ? (
                    <div className="flex flex-col gap-2" aria-label="Loading containers">
                      <Skeleton className="h-9 w-full" />
                      <Skeleton className="h-9 w-full" />
                      <Skeleton className="h-9 w-3/4" />
                    </div>
                  ) : subTargets.isError ? (
                    <Alert variant="destructive">
                      <AlertCircle aria-hidden="true" />
                      <AlertTitle>Could not load containers</AlertTitle>
                      <AlertDescription>
                        {describeConnectorError(subTargets.error)}
                      </AlertDescription>
                    </Alert>
                  ) : (
                    <SearchablePickerList
                      options={subTargets.data}
                      searchLabel="Search containers"
                      emptyMessage="No containers found"
                      selectedId={targetId}
                      disabled={create.isPending}
                      onSelect={(next) => {
                        setTargetId(next);
                        create.reset();
                      }}
                    />
                  )}
                </div>
              ) : (
                <>
                  <div>
                    <h3 className="text-sm font-medium">Widgets</h3>
                    <p className="text-xs text-muted-foreground">
                      Pre-filled from what {detail.data.metadata.name} suggests. The card starts
                      at {detail.data.metadata.minSize[0]}×{detail.data.metadata.minSize[1]} grid
                      units, its smallest readable size.
                    </p>
                  </div>
                  <PlacementBindingEditor
                    dataPoints={detail.data.dataPoints}
                    actions={detail.data.actions}
                    targetId={null}
                    value={bindings}
                    onChange={setBindings}
                    disabled={create.isPending}
                  />
                </>
              )}
            </div>
          )}

          {create.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(create.error)}</AlertDescription>
            </Alert>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={create.isPending}
              onClick={() => {
                reset();
                onOpenChange(false);
              }}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                instanceId === null ||
                detail.data === undefined ||
                create.isPending ||
                (mode === "target" && targetId === null)
              }
            >
              {create.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Add to dashboard
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
