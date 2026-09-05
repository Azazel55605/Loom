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
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { GenericIconPicker } from "@loom/ui-kit/components/GenericIconPicker";
import { PlacementBindingEditor } from "@loom/ui-kit/components/PlacementBindingEditor";
import {
  isPlacementActionComplete,
  PlacementActionEditor,
} from "@loom/ui-kit/components/PlacementActionEditor";
import {
  SearchablePickerList,
  type SearchablePickerOption,
} from "@loom/ui-kit/components/SearchablePickerList";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import type {
  DashboardPlacement,
  DashboardPlacementGroup,
  PlacementAction,
  SubTarget,
  WidgetBinding,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { describeTargetKind } from "@loom/ui-kit/lib/target-label";

type PlacementMode = "server" | "target";

/** Which sort of tile is being added. */
type TileKind = "connector" | "button";

/**
 * A button tile's footprint when it is first placed.
 *
 * It has no connector, so there is no `metadata.minSize` to ask. One column by
 * one row is the smallest thing the grid draws and is enough for an icon and a
 * word; it is draggable and resizable the moment it appears, like every other
 * tile.
 */
const BUTTON_TILE_SIZE = { width: 1, height: 1 } as const;

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
 * Adds a tile to a dashboard.
 *
 * ## Two kinds of tile, one dialog
 *
 * **Connector widget** is the original flow, unchanged: pick an instance, pick
 * a view, edit the widgets pre-filled from the connector's own `defaultLayout`.
 * Size defaults to `metadata.minSize`, which is also the floor the grid and the
 * backend enforce, so a new card is never born too small to read.
 *
 * **Button** is a tile with no connector at all — a label, an icon, and a
 * click. It exists because "go to the network dashboard" and "restart the media
 * server" are things people want on a dashboard that have no reading to show,
 * and giving them a connector they do not use would have stored a lie in the
 * row. The backend accordingly requires a `placementAction` on any placement
 * with no `connectorInstanceId`, which is why the action editor here has no
 * toggle: there is nothing to toggle it to.
 *
 * One dialog rather than two entries in the toolbar, because the choice is
 * "what should this tile be", which is the first question either flow asks.
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
  const [kind, setKind] = React.useState<TileKind>("connector");
  const [instanceId, setInstanceId] = React.useState<string | null>(null);
  const [bindings, setBindings] = React.useState<WidgetBinding[]>([]);
  const [mode, setMode] = React.useState<PlacementMode>("server");
  const [targetId, setTargetId] = React.useState<string | null>(null);
  const [buttonLabel, setButtonLabel] = React.useState("");
  const [buttonIcon, setButtonIcon] = React.useState<string | null>(null);
  const [action, setAction] = React.useState<PlacementAction | null>(null);
  // Which instance's default layout has already been copied in, so re-renders
  // do not overwrite edits but choosing a different connector does.
  const seededFor = React.useRef<string | null>(null);

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: open && kind === "connector",
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
    if (kind !== "connector" || detail.data === undefined) return;
    if (seededFor.current === detail.data.id) return;
    seededFor.current = detail.data.id;
    setMode("server");
    setTargetId(null);
    setBindings(detail.data.defaultLayout.bindings);
  }, [detail.data, kind]);

  // Exactly one connector is the common case in a small deployment, and making
  // someone pick from a list of one is friction with no decision in it.
  React.useEffect(() => {
    if (!open || kind !== "connector" || instanceId !== null) return;
    const only = instances.data?.length === 1 ? instances.data[0] : undefined;
    if (only !== undefined) setInstanceId(only.id);
  }, [instances.data, instanceId, kind, open]);

  const create = useMutation({
    mutationFn: () => {
      const positionY = nextFreeRow(existingPlacements, existingPlacementGroups);

      if (kind === "button") {
        if (action === null) throw new Error("a button tile needs an action");
        return api.createDashboardPlacement(dashboardId, {
          // Null on purpose, and the whole point of this branch: there is no
          // connector, so there is no target and nothing to bind.
          connectorInstanceId: null,
          positionX: 0,
          positionY,
          width: BUTTON_TILE_SIZE.width,
          height: BUTTON_TILE_SIZE.height,
          label: buttonLabel.trim(),
          icon: buttonIcon,
          placementAction: action,
        });
      }

      const connector = detail.data;
      if (connector === undefined) throw new Error("no connector selected");
      const [width, height] = connector.metadata.minSize;
      return api.createDashboardPlacement(dashboardId, {
        connectorInstanceId: connector.id,
        targetId: mode === "target" ? targetId : null,
        positionX: 0,
        positionY,
        width,
        height,
        // The detail endpoint publishes the host layout only. For a sub-target
        // the backend already applies `default_layout_for(targetId)` whenever
        // bindings are omitted, so it remains the one source of the initial
        // container layout rather than a second preview contract being added.
        ...(mode === "server" ? { widgetBindings: bindings } : {}),
        // Optional here: a connector tile may also be clickable, which is what
        // composing the action onto an ordinary placement buys.
        ...(action === null ? {} : { placementAction: action }),
      });
    },
    onSuccess: async () => {
      await onCreated();
      reset();
      onOpenChange(false);
    },
  });

  /** Whether the form holds enough to submit. */
  const submittable =
    kind === "button"
      ? buttonLabel.trim() !== "" && isPlacementActionComplete(action)
      : instanceId !== null &&
        detail.data !== undefined &&
        (mode === "server" || targetId !== null) &&
        (action === null || isPlacementActionComplete(action));

  function reset() {
    setKind("connector");
    setInstanceId(null);
    setBindings([]);
    setMode("server");
    setTargetId(null);
    setButtonLabel("");
    setButtonIcon(null);
    setAction(null);
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
          <DialogTitle>Add a tile</DialogTitle>
          <DialogDescription>
            Show a connector, or add a button that goes somewhere or runs one
            action. You can rearrange and resize it afterwards.
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
            if (submittable) create.mutate();
          }}
        >
          <div className="flex flex-col gap-2">
            <Label>Tile</Label>
            <SegmentedControl
              label="Tile kind"
              value={kind}
              options={[
                { value: "connector", label: "Connector widget" },
                { value: "button", label: "Button" },
              ]}
              onChange={(next) => {
                setKind(next);
                // Nothing carries across: a connector's bindings mean nothing
                // to a button, and a button's label means nothing to a
                // connector tile that already has a name.
                setInstanceId(null);
                setBindings([]);
                setMode("server");
                setTargetId(null);
                setAction(null);
                seededFor.current = null;
                create.reset();
              }}
            />
          </div>

          {kind === "button" ? (
            <div className="flex flex-col gap-5">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="placement-button-label">Label</Label>
                <Input
                  id="placement-button-label"
                  value={buttonLabel}
                  disabled={create.isPending}
                  placeholder="Network"
                  onChange={(event) => setButtonLabel(event.target.value)}
                />
                <p className="text-xs text-muted-foreground">
                  What the tile says. It has no connector to take a name from.
                </p>
              </div>

              <div className="flex flex-col gap-2">
                <Label>Icon</Label>
                <GenericIconPicker
                  value={buttonIcon}
                  defaultIcon={null}
                  label="Button tile icon"
                  defaultLabel="No icon"
                  disabled={create.isPending}
                  onChange={setButtonIcon}
                />
              </div>

              <PlacementActionEditor
                value={action}
                onChange={setAction}
                currentDashboardId={dashboardId}
                // No toggle: the backend refuses a placement with neither a
                // connector nor an action, and offering to turn this off would
                // be offering to build a tile that cannot be saved.
                required
                disabled={create.isPending}
              />
            </div>
          ) : (
          <>
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
                      { value: "target", label: "Specific target" },
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
                    <h3 className="text-sm font-medium">Specific target</h3>
                    <p className="text-xs text-muted-foreground">
                      Choose one view inside {detail.data.metadata.name}. Its recommended
                      widgets will be added automatically.
                    </p>
                  </div>
                  {subTargets.isPending ? (
                    <div className="flex flex-col gap-2" aria-label="Loading views">
                      <Skeleton className="h-9 w-full" />
                      <Skeleton className="h-9 w-full" />
                      <Skeleton className="h-9 w-3/4" />
                    </div>
                  ) : subTargets.isError ? (
                    <Alert variant="destructive">
                      <AlertCircle aria-hidden="true" />
                      <AlertTitle>Could not load views</AlertTitle>
                      <AlertDescription>
                        {describeConnectorError(subTargets.error)}
                      </AlertDescription>
                    </Alert>
                  ) : (
                    <SearchablePickerList
                      // Badged by the connector's own word for what each entry
                      // is, so a mixed list reads as one. Search still matches
                      // `label` alone: the badge is a property of the row, not
                      // another thing to type at.
                      options={pickerOptions(subTargets.data)}
                      searchLabel="Search targets"
                      emptyMessage="No targets found"
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

          {instanceId === null || detail.data === undefined ? null : (
            <div className="flex flex-col gap-2 border-t pt-4">
              <h3 className="text-sm font-medium">Click behaviour</h3>
              <PlacementActionEditor
                value={action}
                onChange={setAction}
                currentDashboardId={dashboardId}
                disabled={create.isPending}
              />
            </div>
          )}
          </>
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
            <Button type="submit" disabled={!submittable || create.isPending}>
              {create.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Add to dashboard
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/**
 * One sub-target per picker row, tagged with what sort of thing it is.
 *
 * A `kind` this client has never seen still gets a badge, title-cased — the
 * vocabulary belongs to the connector, and dropping an unrecognised word would
 * make a mixed list read as though the entries were interchangeable.
 */
function pickerOptions(targets: SubTarget[]): SearchablePickerOption[] {
  return targets.map((target) => ({
    id: target.id,
    label: target.label,
    icon: target.icon,
    badge: describeTargetKind(target.kind) ?? undefined,
  }));
}
