import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Label } from "@loom/ui-kit/components/ui/label";
import { ButtonTileWizard } from "@loom/ui-kit/components/ButtonTileWizard";
import { PlacementBindingEditor } from "@loom/ui-kit/components/PlacementBindingEditor";
import {
  isPlacementActionComplete,
  PlacementActionEditor,
} from "@loom/ui-kit/components/PlacementActionEditor";
import type {
  DashboardPlacement,
  PlacementAction,
  WidgetBinding,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/**
 * Edits the widgets on a placement that already exists.
 *
 * The other half of `AddPlacementDialog`, sharing its editor: this one loads the
 * stored `widgetBindings` instead of the connector's default layout, and PATCHes
 * only that field — position and size belong to the grid, and sending them from
 * here would undo a drag that happened while the dialog was open.
 *
 * Which connector a placement points at is deliberately **not** editable. The
 * backend fixes it on purpose (see `UpdateDashboardPlacementRequest`), and
 * re-pointing a card would silently invalidate every binding on it; deleting and
 * adding is the honest form of that operation.
 *
 * ## Click behaviour is a second, independent section
 *
 * A `placementAction` composes onto any placement, so this dialog edits it
 * alongside the bindings rather than in a dialog of its own. It is off by
 * default and leaving it off changes nothing about the widgets — the two
 * sections do not interact, which is exactly why click behaviour was made a
 * field on an ordinary placement rather than a kind of placement.
 *
 * A **static tile** has no bindings to edit, so only that section is shown, and
 * its action cannot be turned off: the backend refuses a placement with neither
 * a connector nor an action.
 */
export function PlacementBindingsDialog({
  dashboardId,
  placement,
  onOpenChange,
  onSaved,
}: {
  dashboardId: string;
  /** The placement being edited, or `null` to render nothing. */
  placement: DashboardPlacement | null;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void | Promise<void>;
}) {
  const api = useApiClient();
  const [bindings, setBindings] = React.useState<WidgetBinding[]>([]);
  const [action, setAction] = React.useState<PlacementAction | null>(null);
  const [label, setLabel] = React.useState("");
  const [icon, setIcon] = React.useState<string | null>(null);
  const seededFor = React.useRef<string | null>(null);

  const instanceId = placement?.connector?.id ?? null;
  const isStaticTile = placement !== null && placement.connector === null;

  const detail = useQuery({
    queryKey: ["connector-instance", instanceId, placement?.targetId ?? null],
    queryFn: ({ signal }) =>
      api.getConnectorInstance(instanceId as string, placement?.targetId ?? null, signal),
    enabled: instanceId !== null,
  });
  const resourceKinds = useQuery({
    queryKey: ["connector-resource-kinds", instanceId, placement?.targetId ?? null],
    queryFn: ({ signal }) =>
      api.getResourceKinds(instanceId as string, placement?.targetId ?? null, signal),
    enabled: instanceId !== null,
  });

  React.useEffect(() => {
    if (placement === null) return;
    if (seededFor.current === placement.id) return;
    seededFor.current = placement.id;
    setBindings(placement.widgetBindings);
    setAction(placement.placementAction);
    setLabel(placement.label ?? "");
    setIcon(placement.icon);
  }, [placement]);

  const save = useMutation({
    mutationFn: () => {
      if (placement === null) throw new Error("no placement selected");
      return api.updateDashboardPlacement(dashboardId, placement.id, {
        // A static tile has nothing to bind, and sending an empty array would
        // be indistinguishable from clearing bindings it never had.
        ...(isStaticTile ? {} : { widgetBindings: bindings }),
        ...(isStaticTile ? { label: label.trim(), icon } : {}),
        // Always sent, including as `null`: absent would leave the stored
        // action alone, which is not what turning the toggle off means.
        placementAction: action,
      });
    },
    onSuccess: async () => {
      await onSaved();
      onOpenChange(false);
    },
  });

  if (placement === null) return null;

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) {
          seededFor.current = null;
          save.reset();
        }
      }}
    >
      <DialogContent className="flex max-h-[85dvh] max-w-2xl flex-col overflow-hidden">
        <DialogHeader>
          <DialogTitle>
            {isStaticTile
              ? `Edit ${placement.label ?? "button"}`
              : `Widgets on ${placement.connector?.name ?? "this placement"}`}
          </DialogTitle>
          <DialogDescription>
            {isStaticTile
              ? "Choose what this button does when it is clicked. Its position and size are set by dragging it on the dashboard."
              : "Choose what this card shows, and optionally what clicking it does. Its position and size are set by dragging it on the dashboard."}
          </DialogDescription>
        </DialogHeader>

        <form
          className="flex min-h-0 flex-1 flex-col gap-5"
          // See AddPlacementDialog: native bubbles would pre-empt the backend's
          // own message about a binding it refused.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            save.mutate();
          }}
        >
          {isStaticTile ? (
            <ButtonTileWizard
              name={label}
              icon={icon}
              action={action}
              currentDashboardId={dashboardId}
              disabled={save.isPending}
              pending={save.isPending}
              submitLabel="Save button"
              resetKey={placement.id}
              onNameChange={setLabel}
              onIconChange={setIcon}
              onActionChange={setAction}
              onCancel={() => onOpenChange(false)}
              error={
                save.isError ? (
                  <Alert variant="destructive" className="mt-4">
                    <AlertCircle aria-hidden="true" />
                    <AlertDescription>{describeConnectorError(save.error)}</AlertDescription>
                  </Alert>
                ) : undefined
              }
            />
          ) : (
            <>
              <div className="min-h-0 flex-1 overflow-y-auto pr-1">
              {/* Target identity is intentionally read-only in this pass.
                  Changing it would require discarding and re-seeding the entire
                  binding set; delete and recreate the placement instead. */}
              <div className="flex flex-col gap-2">
                <Label>View</Label>
                <div>
                  <Badge variant="secondary">
                    {placement.targetId === null ? "Server info" : placement.targetId}
                  </Badge>
                </div>
              </div>

              {detail.isPending || resourceKinds.isPending ? (
                <Skeleton className="h-32 w-full" />
              ) : detail.isError ? (
                <Alert variant="destructive">
                  <AlertCircle aria-hidden="true" />
                  <AlertDescription>
                    {describeConnectorError(detail.error)}
                  </AlertDescription>
                </Alert>
              ) : (
                <PlacementBindingEditor
                  connectorInstanceId={instanceId as string}
                  dataPoints={detail.data.dataPoints}
                  actions={detail.data.actions}
                  resourceKinds={resourceKinds.data ?? []}
                  supportsBrowsableContent={detail.data.supportsBrowsableContent}
                  supportsMediaSource={detail.data.supportsMediaSource}
                  supportsMediaTarget={detail.data.supportsMediaTarget}
                  targetId={placement.targetId}
                  value={bindings}
                  onChange={setBindings}
                  disabled={save.isPending}
                />
              )}
                <div className="mt-5 flex flex-col gap-2 border-t pt-4">
                  <h3 className="text-sm font-medium">Click behaviour</h3>
                  <PlacementActionEditor
                    value={action}
                    onChange={setAction}
                    currentDashboardId={dashboardId}
                    disabled={save.isPending}
                  />
                </div>

                {save.isError ? (
                  <Alert variant="destructive" className="mt-5">
                    <AlertCircle aria-hidden="true" />
                    <AlertDescription>{describeConnectorError(save.error)}</AlertDescription>
                  </Alert>
                ) : null}
              </div>

              <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={save.isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                detail.isPending ||
                save.isPending ||
                // A half-filled action is refused by the backend; saying so
                // with a dead button beats saying so with a 400.
                (action !== null && !isPlacementActionComplete(action))
              }
            >
              {save.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Save tile
            </Button>
              </DialogFooter>
            </>
          )}
        </form>
      </DialogContent>
    </Dialog>
  );
}
