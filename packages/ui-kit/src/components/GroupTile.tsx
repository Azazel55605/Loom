import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { GripVertical, Ungroup } from "lucide-react";
import { toast } from "sonner";

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
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@loom/ui-kit/components/ui/card";
import {
  DRAG_HANDLE_CLASS,
  PlacementTile,
  type LiveStatus,
} from "@loom/ui-kit/components/PlacementTile";
import type { DashboardPlacement, DashboardPlacementGroup } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * A visual placement group. The outer card is one grid item; member placements
 * remain ordinary live `PlacementTile`s in a wrapping row inside it.
 *
 * The grid minimum width is computed by `DashboardView`: member connector
 * minimum widths are summed and capped at the six-column desktop grid. This
 * keeps pairs comfortably side by side while allowing larger groups to wrap
 * instead of demanding a grid wider than the dashboard itself.
 */
export function GroupTile({
  dashboardId,
  group,
  live,
  editing,
  onEditBindings,
  onChanged,
}: {
  dashboardId: string;
  group: DashboardPlacementGroup;
  live: Record<string, LiveStatus>;
  editing: boolean;
  onEditBindings: (placement: DashboardPlacement) => void;
  onChanged: () => void | Promise<void>;
}) {
  const api = useApiClient();
  const [splitOpen, setSplitOpen] = React.useState(false);

  const reorder = useMutation({
    mutationFn: (memberOrder: string[]) =>
      api.updateDashboardPlacementGroup(dashboardId, group.id, { memberOrder }),
    onSuccess: onChanged,
    onError: (error) => {
      toast.error("Could not reorder the group", {
        description: describeConnectorError(error),
      });
    },
  });

  const removeMember = useMutation({
    mutationFn: (placementId: string) =>
      api.deleteDashboardPlacementGroupMember(dashboardId, group.id, placementId),
    onSuccess: onChanged,
    onError: (error) => {
      toast.error("Could not remove the tile from its group", {
        description: describeConnectorError(error),
      });
    },
  });

  const split = useMutation({
    mutationFn: () => api.deleteDashboardPlacementGroup(dashboardId, group.id),
    onSuccess: async () => {
      await onChanged();
      setSplitOpen(false);
    },
    onError: (error) => {
      toast.error("Could not split the group", {
        description: describeConnectorError(error),
      });
    },
  });

  function moveMember(index: number, offset: -1 | 1) {
    const nextIndex = index + offset;
    if (nextIndex < 0 || nextIndex >= group.members.length) return;
    const memberOrder = group.members.map((member) => member.id);
    [memberOrder[index], memberOrder[nextIndex]] = [memberOrder[nextIndex], memberOrder[index]];
    reorder.mutate(memberOrder);
  }

  const memberMutationPending = reorder.isPending || removeMember.isPending;

  return (
    <>
      <Card className="flex h-full min-h-0 flex-col overflow-hidden">
        <CardHeader
          className={cn(
            "flex-row items-center justify-between gap-3 space-y-0 px-4 py-3",
            editing && `${DRAG_HANDLE_CLASS} cursor-grab active:cursor-grabbing`,
          )}
        >
          <div className="flex min-w-0 items-center gap-2">
            {editing ? (
              <GripVertical className="shrink-0 text-muted-foreground" aria-hidden="true" />
            ) : null}
            <div className="min-w-0">
              <CardTitle className="truncate text-sm">Group of {group.members.length}</CardTitle>
              <p className="truncate text-xs text-muted-foreground">
                {group.members.map((member) => member.connector.name).join(" · ")}
              </p>
            </div>
          </div>
          {editing ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="loom-grid-control shrink-0"
              disabled={split.isPending}
              aria-label="Split this group apart"
              onClick={() => setSplitOpen(true)}
            >
              <Ungroup data-icon="inline-start" aria-hidden="true" />
              <span className="hidden sm:inline">Split apart</span>
            </Button>
          ) : null}
        </CardHeader>

        <CardContent className="flex min-h-0 flex-1 flex-wrap content-start gap-3 overflow-auto px-3 pb-3">
          {group.members.map((member, index) => (
            <div key={member.id} className="min-h-[12rem] min-w-[16rem] flex-1 basis-[16rem]">
              <PlacementTile
                placement={member}
                live={live[member.connector.id]}
                editing={editing}
                onEditBindings={onEditBindings}
                groupMember={
                  editing
                    ? {
                        index,
                        total: group.members.length,
                        pending: memberMutationPending,
                        onMoveLeft: () => moveMember(index, -1),
                        onMoveRight: () => moveMember(index, 1),
                        onRemove: () => removeMember.mutate(member.id),
                      }
                    : undefined
                }
              />
            </div>
          ))}
        </CardContent>
      </Card>

      <AlertDialog
        open={splitOpen}
        onOpenChange={(open) => {
          setSplitOpen(open);
          if (!open) split.reset();
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Split this group apart?</AlertDialogTitle>
            <AlertDialogDescription>
              All {group.members.length} tiles return to their saved standalone positions. No
              placement or connector is deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={split.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={split.isPending}
              onClick={(event) => {
                event.preventDefault();
                split.mutate();
              }}
            >
              {split.isPending ? "Splitting…" : "Split apart"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
