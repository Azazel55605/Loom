import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
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
import type { DashboardPlacement, WidgetBinding } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/**
 * The next free row, given what is already placed.
 *
 * Deliberately crude: everything new lands at the left edge, below the lowest
 * existing card. Anything cleverer would be guessing at an arrangement the user
 * is about to change anyway — the card is draggable the moment it appears, and
 * a placement that turned up in a surprising gap would be harder to find than
 * one that turned up at the bottom.
 */
function nextFreeRow(placements: DashboardPlacement[]): number {
  return placements.reduce(
    (lowest, placement) => Math.max(lowest, placement.positionY + placement.height),
    0,
  );
}

/**
 * Adds a connector to a dashboard.
 *
 * One dialog rather than a wizard: choosing the instance reveals its widgets
 * below, pre-filled from the connector's own `defaultLayout`, and both are
 * submitted together. The default layout is a *suggestion* — the whole editor
 * is live before anything is created, so a user who wants three of the six
 * widgets deletes three rows rather than placing the card and pruning it after.
 *
 * Size defaults to the connector's `metadata.minSize`, which is also the floor
 * the grid and the backend enforce, so a new card is never born too small to
 * read.
 */
export function AddPlacementDialog({
  dashboardId,
  existingPlacements,
  open,
  onOpenChange,
  onCreated,
}: {
  dashboardId: string;
  existingPlacements: DashboardPlacement[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void | Promise<void>;
}) {
  const api = useApiClient();
  const [instanceId, setInstanceId] = React.useState<string | null>(null);
  const [bindings, setBindings] = React.useState<WidgetBinding[]>([]);
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

  React.useEffect(() => {
    if (detail.data === undefined) return;
    if (seededFor.current === detail.data.id) return;
    seededFor.current = detail.data.id;
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
        positionX: 0,
        positionY: nextFreeRow(existingPlacements),
        width,
        height,
        widgetBindings: bindings,
      });
    },
    onSuccess: async () => {
      await onCreated();
      onOpenChange(false);
    },
  });

  function reset() {
    setInstanceId(null);
    setBindings([]);
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
            Choose a connector instance and the widgets to show for it. You can rearrange and
            resize the card afterwards.
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-5"
          // Native validation bubbles are a browser-default control per
          // docs/UI_GUIDELINES.md, and they would pre-empt the backend's own
          // message about a binding or a size it refused.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            if (instanceId !== null) create.mutate();
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
            <div className="space-y-3">
              <div>
                <h3 className="text-sm font-medium">Widgets</h3>
                <p className="text-xs text-muted-foreground">
                  Pre-filled from what {detail.data.metadata.name} suggests. The card starts at{" "}
                  {detail.data.metadata.minSize[0]}×{detail.data.metadata.minSize[1]} grid units,
                  its smallest readable size.
                </p>
              </div>
              <PlacementBindingEditor
                dataPoints={detail.data.dataPoints}
                actions={detail.data.actions}
                value={bindings}
                onChange={setBindings}
                disabled={create.isPending}
              />
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
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={instanceId === null || create.isPending}>
              {create.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Add to dashboard
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
