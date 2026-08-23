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
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { PlacementBindingEditor } from "@loom/ui-kit/components/PlacementBindingEditor";
import type { DashboardPlacement, WidgetBinding } from "@loom/ui-kit/lib/api";
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
  const seededFor = React.useRef<string | null>(null);

  const instanceId = placement?.connector.id ?? null;

  const detail = useQuery({
    queryKey: ["connector-instance", instanceId],
    queryFn: ({ signal }) => api.getConnectorInstance(instanceId as string, signal),
    enabled: instanceId !== null,
  });

  React.useEffect(() => {
    if (placement === null) return;
    if (seededFor.current === placement.id) return;
    seededFor.current = placement.id;
    setBindings(placement.widgetBindings);
  }, [placement]);

  const save = useMutation({
    mutationFn: () => {
      if (placement === null) throw new Error("no placement selected");
      return api.updateDashboardPlacement(dashboardId, placement.id, {
        widgetBindings: bindings,
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
      <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Widgets on {placement.connector.name}</DialogTitle>
          <DialogDescription>
            Choose what this card shows. Its position and size are set by dragging it on the
            dashboard.
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-5"
          // See AddPlacementDialog: native bubbles would pre-empt the backend's
          // own message about a binding it refused.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            save.mutate();
          }}
        >
          {detail.isPending ? (
            <Skeleton className="h-32 w-full" />
          ) : detail.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(detail.error)}</AlertDescription>
            </Alert>
          ) : (
            <PlacementBindingEditor
              dataPoints={detail.data.dataPoints}
              actions={detail.data.actions}
              value={bindings}
              onChange={setBindings}
              disabled={save.isPending}
            />
          )}

          {save.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(save.error)}</AlertDescription>
            </Alert>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={save.isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={detail.isPending || save.isPending}>
              {save.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Save widgets
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
