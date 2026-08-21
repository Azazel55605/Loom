import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Trash2, Users } from "lucide-react";

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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import type {
  DashboardShare,
  DashboardShareRole,
  DashboardShareTargetType,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

const shareQueryKey = (dashboardId: string) => ["dashboard-shares", dashboardId] as const;

/** Owner-only share management for a dashboard. */
export function DashboardSharesDialog({
  dashboardId,
  dashboardName,
  open,
  onOpenChange,
}: {
  dashboardId: string;
  dashboardName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [targetType, setTargetType] = React.useState<DashboardShareTargetType>("user");
  const [targetId, setTargetId] = React.useState("");
  const [role, setRole] = React.useState<DashboardShareRole>("view");
  const [removing, setRemoving] = React.useState<DashboardShare | null>(null);

  const shares = useQuery({
    queryKey: shareQueryKey(dashboardId),
    queryFn: ({ signal }) => api.getDashboardShares(dashboardId, signal),
    enabled: open,
  });
  const users = useQuery({
    queryKey: ["users"],
    queryFn: ({ signal }) => api.getUsers(signal),
    enabled: open && targetType === "user",
  });
  const groups = useQuery({
    queryKey: ["groups"],
    queryFn: ({ signal }) => api.getGroups(signal),
    enabled: open && targetType === "group",
  });

  const addShare = useMutation({
    mutationFn: () => api.addDashboardShare(dashboardId, { targetType, targetId, role }),
    onSuccess: (share) => {
      queryClient.setQueryData<DashboardShare[]>(shareQueryKey(dashboardId), (current) =>
        current === undefined ? [share] : [...current, share],
      );
      setTargetId("");
    },
  });
  const removeShare = useMutation({
    mutationFn: (share: DashboardShare) => api.removeDashboardShare(dashboardId, share.id),
    onSuccess: (_result, share) => {
      queryClient.setQueryData<DashboardShare[]>(shareQueryKey(dashboardId), (current) =>
        current?.filter((item) => item.id !== share.id),
      );
      setRemoving(null);
    },
  });

  const targets =
    targetType === "user"
      ? users.data?.map((user) => ({ id: user.id, name: user.username }))
      : groups.data?.map((group) => ({ id: group.id, name: group.name }));
  const targetsPending = targetType === "user" ? users.isPending : groups.isPending;
  const targetsError = targetType === "user" ? users.error : groups.error;

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          onOpenChange(next);
          if (!next) {
            addShare.reset();
            removeShare.reset();
            setRemoving(null);
          }
        }}
      >
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>Share {dashboardName}</DialogTitle>
            <DialogDescription>
              Give a user or group view-only or editing access to this dashboard.
            </DialogDescription>
          </DialogHeader>

          <section className="flex flex-col gap-3" aria-labelledby="current-shares-heading">
            <h3 id="current-shares-heading" className="text-sm font-semibold">
              Current shares
            </h3>
            {shares.isPending ? (
              <div className="flex flex-col gap-2">
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-full" />
              </div>
            ) : null}
            {shares.isError ? (
              <Alert variant="destructive">
                <AlertCircle aria-hidden="true" />
                <AlertTitle>Could not load shares</AlertTitle>
                <AlertDescription>{describeConnectorError(shares.error)}</AlertDescription>
              </Alert>
            ) : null}
            {shares.isSuccess && shares.data.length === 0 ? (
              <p className="text-sm text-muted-foreground">This dashboard is not shared yet.</p>
            ) : null}
            {shares.isSuccess && shares.data.length > 0 ? (
              <ul className="flex flex-col gap-2">
                {shares.data.map((share) => (
                  <li
                    key={share.id}
                    className="surface-panel flex items-center gap-3 rounded-md border p-3"
                  >
                    <Users aria-hidden="true" className="text-muted-foreground" />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">{share.resolvedName}</p>
                      <p className="text-xs capitalize text-muted-foreground">
                        {share.targetType}
                      </p>
                    </div>
                    <Badge variant="outline">{share.role === "edit" ? "Edit" : "View"}</Badge>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      aria-label={`Remove share for ${share.resolvedName}`}
                      onClick={() => {
                        removeShare.reset();
                        setRemoving(share);
                      }}
                    >
                      <Trash2 aria-hidden="true" />
                    </Button>
                  </li>
                ))}
              </ul>
            ) : null}
          </section>

          <section className="flex flex-col gap-4 border-t pt-4" aria-labelledby="add-share-heading">
            <h3 id="add-share-heading" className="text-sm font-semibold">
              Add share
            </h3>
            <form
              className="flex flex-col gap-4"
              onSubmit={(event) => {
                event.preventDefault();
                if (targetId) addShare.mutate();
              }}
            >
              <div className="flex flex-col gap-2">
                <Label>Target type</Label>
                <SegmentedControl
                  label="Share target type"
                  value={targetType}
                  onChange={(next) => {
                    setTargetType(next);
                    setTargetId("");
                    addShare.reset();
                  }}
                  options={[
                    { value: "user", label: "User" },
                    { value: "group", label: "Group" },
                  ]}
                />
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                <div className="flex flex-col gap-2">
                  <Label htmlFor="dashboard-share-target">
                    {targetType === "user" ? "User" : "Group"}
                  </Label>
                  <Select value={targetId} onValueChange={setTargetId}>
                    <SelectTrigger id="dashboard-share-target" disabled={targetsPending}>
                      <SelectValue
                        placeholder={targetsPending ? "Loading…" : `Select ${targetType}`}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {targets?.map((target) => (
                          <SelectItem key={target.id} value={target.id}>
                            {target.name}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>

                <div className="flex flex-col gap-2">
                  <Label htmlFor="dashboard-share-role">Role</Label>
                  <Select value={role} onValueChange={(value) => setRole(value as DashboardShareRole)}>
                    <SelectTrigger id="dashboard-share-role">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="view">View</SelectItem>
                        <SelectItem value="edit">Edit</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {targetsError ? (
                <Alert variant="destructive">
                  <AlertCircle aria-hidden="true" />
                  <AlertTitle>Could not load {targetType}s</AlertTitle>
                  <AlertDescription>{describeConnectorError(targetsError)}</AlertDescription>
                </Alert>
              ) : null}
              {addShare.isError ? (
                <Alert variant="destructive">
                  <AlertCircle aria-hidden="true" />
                  <AlertTitle>Could not add share</AlertTitle>
                  <AlertDescription>{describeConnectorError(addShare.error)}</AlertDescription>
                </Alert>
              ) : null}

              <Button
                type="submit"
                className="self-end"
                disabled={!targetId || targetsPending || addShare.isPending}
              >
                {addShare.isPending ? "Adding…" : "Add share"}
              </Button>
            </form>
          </section>
        </DialogContent>
      </Dialog>

      <AlertDialog
        open={removing !== null}
        onOpenChange={(next) => {
          if (!next) {
            removeShare.reset();
            setRemoving(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this share?</AlertDialogTitle>
            <AlertDialogDescription>
              {removing === null
                ? "This access will be revoked immediately."
                : `${removing.resolvedName} will immediately lose this shared access.`}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {removeShare.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>{describeConnectorError(removeShare.error)}</AlertDescription>
            </Alert>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removeShare.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={removing === null || removeShare.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (removing !== null) removeShare.mutate(removing);
              }}
            >
              {removeShare.isPending ? "Removing…" : "Remove share"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
