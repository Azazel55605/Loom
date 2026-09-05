import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { GripVertical, Pencil, Ungroup } from "lucide-react";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { GenericIconPicker } from "@loom/ui-kit/components/GenericIconPicker";
import {
  DRAG_HANDLE_CLASS,
  PlacementTile,
  type LiveStatus,
} from "@loom/ui-kit/components/PlacementTile";
import type { DashboardPlacement, DashboardPlacementGroup } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";
import { describeTarget } from "@loom/ui-kit/lib/target-label";

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
  onNavigateDashboard,
  onChanged,
}: {
  dashboardId: string;
  group: DashboardPlacementGroup;
  live: Record<string, LiveStatus>;
  editing: boolean;
  onEditBindings: (placement: DashboardPlacement) => void;
  /** Passed through to members: a grouped tile is still clickable if it was
   *  clickable standing alone. See `PlacementTile`. */
  onNavigateDashboard?: (dashboardId: string) => void;
  onChanged: () => void | Promise<void>;
}) {
  const api = useApiClient();
  const [splitOpen, setSplitOpen] = React.useState(false);
  const [editOpen, setEditOpen] = React.useState(false);
  const [name, setName] = React.useState(group.name);
  const [icon, setIcon] = React.useState<string | null>(group.icon);

  React.useEffect(() => {
    if (!editOpen) return;
    setName(group.name);
    setIcon(group.icon);
  }, [editOpen, group.icon, group.name]);

  const updateIdentity = useMutation({
    mutationFn: () =>
      api.updateDashboardPlacementGroup(dashboardId, group.id, {
        name: name.trim(),
        icon,
      }),
    onSuccess: async () => {
      await onChanged();
      setEditOpen(false);
    },
    onError: (error) => {
      toast.error("Could not update the group", {
        description: describeConnectorError(error),
      });
    },
  });

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
            <ConnectorIcon
              typeIcon="lucide:boxes"
              iconOverride={group.icon}
              size={20}
              className="shrink-0"
            />
            <div className="min-w-0">
              <CardTitle className="truncate text-sm">{group.name}</CardTitle>
              <p className="truncate text-xs text-muted-foreground">
                {group.members
                  .map((member) => {
                    // The same reading a standalone tile gives, so one
                    // placement does not read two ways depending on whether
                    // somebody grouped it.
                    // A static tile has no connector to name it, so it names
                    // itself; the fallback keeps a mixed group readable.
                    const name =
                      member.connector?.name ?? member.label ?? "Button";
                    const target = describeTarget(member.targetId);
                    return target === null ? name : `${name} · ${target.text}`;
                  })
                  .join(" · ")}
              </p>
            </div>
          </div>
          {editing ? (
            <div className="loom-grid-control flex shrink-0 gap-2">
              <Button
                type="button"
                variant="outline"
                size="icon"
                aria-label={`Edit ${group.name}`}
                onClick={() => setEditOpen(true)}
              >
                <Pencil aria-hidden="true" />
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={split.isPending}
                aria-label="Split this group apart"
                onClick={() => setSplitOpen(true)}
              >
                <Ungroup data-icon="inline-start" aria-hidden="true" />
                <span className="hidden sm:inline">Split apart</span>
              </Button>
            </div>
          ) : null}
        </CardHeader>

        <CardContent className="flex min-h-0 flex-1 flex-wrap content-start gap-3 overflow-auto px-3 pb-3">
          {group.members.map((member, index) => (
            <div key={member.id} className="min-h-[12rem] min-w-[16rem] flex-1 basis-[16rem]">
              <PlacementTile
                dashboardId={dashboardId}
                placement={member}
                live={
                  member.connector === null ? undefined : live[member.connector.id]
                }
                editing={editing}
                onEditBindings={onEditBindings}
                onNavigateDashboard={onNavigateDashboard}
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

      <Dialog
        open={editOpen}
        onOpenChange={(open) => {
          setEditOpen(open);
          if (!open) updateIdentity.reset();
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit group</DialogTitle>
            <DialogDescription>
              Give this collection a name and icon that make it easy to find.
            </DialogDescription>
          </DialogHeader>
          <form
            className="space-y-5"
            onSubmit={(event) => {
              event.preventDefault();
              if (name.trim()) updateIdentity.mutate();
            }}
          >
            <div className="space-y-2">
              <Label htmlFor={`group-name-${group.id}`}>Name</Label>
              <Input
                id={`group-name-${group.id}`}
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={updateIdentity.isPending}
                autoFocus
              />
            </div>
            <div className="space-y-2">
              <Label>Icon</Label>
              <GenericIconPicker
                label="Group icon"
                value={icon}
                defaultIcon="lucide:boxes"
                defaultLabel="Use group default"
                onChange={setIcon}
                disabled={updateIdentity.isPending}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={!name.trim() || updateIdentity.isPending}>
                {updateIdentity.isPending ? "Saving…" : "Save changes"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

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
