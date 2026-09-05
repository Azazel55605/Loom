import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Eye, EyeOff, Loader2, Pencil, Trash2, UserRoundCog } from "lucide-react";
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
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent } from "@loom/ui-kit/components/ui/card";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@loom/ui-kit/components/ui/table";
import type { AdminDashboard } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";

/** Instance-wide dashboard administration, independent of dashboard shares. */
export function DashboardsPanel() {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const dashboards = useQuery({
    queryKey: ["admin-dashboards"],
    queryFn: ({ signal }) => api.getAdminDashboards(signal),
    retry: false,
  });
  const users = useQuery({
    queryKey: ["users"],
    queryFn: ({ signal }) => api.getUsers(signal),
    retry: false,
  });
  const [renaming, setRenaming] = React.useState<AdminDashboard | null>(null);
  const [newName, setNewName] = React.useState("");
  const [reassigning, setReassigning] = React.useState<AdminDashboard | null>(null);
  const [newOwnerId, setNewOwnerId] = React.useState("");
  const [deleting, setDeleting] = React.useState<AdminDashboard | null>(null);

  const refresh = React.useCallback(
    () => Promise.all([
      queryClient.invalidateQueries({ queryKey: ["admin-dashboards"] }),
      queryClient.invalidateQueries({ queryKey: ["dashboards"] }),
    ]),
    [queryClient],
  );

  const update = useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: { name?: string; hidden?: boolean; ownerUserId?: string } }) =>
      api.updateAdminDashboard(id, patch),
    onSuccess: async () => {
      setRenaming(null);
      setReassigning(null);
      toast.success("Dashboard updated.");
      await refresh();
    },
    onError: (error: unknown) => toast.error("Could not update dashboard", {
      description: describeAdminFailure(error).message,
    }),
  });
  const remove = useMutation({
    mutationFn: (dashboard: AdminDashboard) => api.deleteAdminDashboard(dashboard.id),
    onSuccess: async (_result, dashboard) => {
      setDeleting(null);
      toast.success(`Deleted ${dashboard.name}.`);
      await refresh();
    },
    onError: (error: unknown) => toast.error("Could not delete dashboard", {
      description: describeAdminFailure(error).message,
    }),
  });

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Administer every dashboard on this instance, including ownership and hidden dashboards.
      </p>
      {dashboards.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load dashboards</AlertTitle>
          <AlertDescription>{describeAdminFailure(dashboards.error).message}</AlertDescription>
        </Alert>
      ) : null}
      {users.isError ? (
        <Alert>
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Owners unavailable</AlertTitle>
          <AlertDescription>
            Dashboard rows remain available, but ownership cannot be reassigned until users load.
          </AlertDescription>
        </Alert>
      ) : null}
      <Card>
        <CardContent className="p-2">
          <div className="max-w-full overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-3">Name</TableHead>
                  <TableHead>Owner</TableHead>
                  <TableHead>Visibility</TableHead>
                  <TableHead>Shares</TableHead>
                  <TableHead>Placements</TableHead>
                  <TableHead className="w-48 pr-3 text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {dashboards.isPending ? <LoadingRows /> : null}
                {dashboards.data?.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                      No dashboards.
                    </TableCell>
                  </TableRow>
                ) : null}
                {dashboards.data?.map((dashboard) => (
                  <TableRow key={dashboard.id}>
                    <TableCell className="pl-3 font-medium">{dashboard.name}</TableCell>
                    <TableCell>{dashboard.ownerUsername}</TableCell>
                    <TableCell>
                      <Badge variant={dashboard.hidden ? "secondary" : "outline"}>
                        {dashboard.hidden ? "Hidden" : "Visible"}
                      </Badge>
                    </TableCell>
                    <TableCell>{dashboard.shareCount}</TableCell>
                    <TableCell>{dashboard.placementCount}</TableCell>
                    <TableCell className="pr-3">
                      <div className="flex justify-end gap-1">
                        <Button variant="ghost" size="icon" aria-label={`Rename ${dashboard.name}`} onClick={() => {
                          setNewName(dashboard.name);
                          setRenaming(dashboard);
                        }}><Pencil aria-hidden="true" /></Button>
                        <Button variant="ghost" size="icon" aria-label={`${dashboard.hidden ? "Show" : "Hide"} ${dashboard.name}`} disabled={update.isPending} onClick={() => update.mutate({ id: dashboard.id, patch: { hidden: !dashboard.hidden } })}>
                          {dashboard.hidden ? <Eye aria-hidden="true" /> : <EyeOff aria-hidden="true" />}
                        </Button>
                        <Button variant="ghost" size="icon" aria-label={`Reassign ${dashboard.name}`} disabled={!users.data} onClick={() => {
                          setNewOwnerId(dashboard.ownerUserId);
                          setReassigning(dashboard);
                        }}><UserRoundCog aria-hidden="true" /></Button>
                        <Button variant="ghost" size="icon" className="text-destructive hover:text-destructive" aria-label={`Delete ${dashboard.name}`} onClick={() => setDeleting(dashboard)}><Trash2 aria-hidden="true" /></Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>

      <Dialog open={renaming !== null} onOpenChange={(open) => !open && setRenaming(null)}>
        <DialogContent>
          <DialogHeader><DialogTitle>Rename dashboard</DialogTitle><DialogDescription>Change the dashboard name for everyone who can access it.</DialogDescription></DialogHeader>
          <div className="grid gap-2"><Label htmlFor="admin-dashboard-name">Name</Label><Input id="admin-dashboard-name" value={newName} onChange={(event) => setNewName(event.target.value)} /></div>
          <DialogFooter><Button variant="outline" onClick={() => setRenaming(null)}>Cancel</Button><Button disabled={update.isPending || newName.trim() === ""} onClick={() => renaming && update.mutate({ id: renaming.id, patch: { name: newName.trim() } })}>{update.isPending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}Rename</Button></DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={reassigning !== null} onOpenChange={(open) => !open && setReassigning(null)}>
        <DialogContent>
          <DialogHeader><DialogTitle>Reassign dashboard</DialogTitle><DialogDescription>The selected user becomes the owner and receives full control of this dashboard.</DialogDescription></DialogHeader>
          <div className="grid gap-2"><Label htmlFor="admin-dashboard-owner">Owner</Label><Select value={newOwnerId} onValueChange={setNewOwnerId}><SelectTrigger id="admin-dashboard-owner"><SelectValue placeholder="Choose a user" /></SelectTrigger><SelectContent>{users.data?.map((user) => <SelectItem key={user.id} value={user.id}>{user.username}</SelectItem>)}</SelectContent></Select></div>
          <DialogFooter><Button variant="outline" onClick={() => setReassigning(null)}>Cancel</Button><Button disabled={update.isPending || newOwnerId === ""} onClick={() => reassigning && update.mutate({ id: reassigning.id, patch: { ownerUserId: newOwnerId } })}>{update.isPending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}Reassign</Button></DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent><AlertDialogHeader><AlertDialogTitle>Delete {deleting?.name}?</AlertDialogTitle><AlertDialogDescription>This permanently deletes the dashboard, its shares, groups, and placements.</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel><AlertDialogAction disabled={remove.isPending} onClick={(event) => { event.preventDefault(); if (deleting) remove.mutate(deleting); }}>{remove.isPending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}Delete dashboard</AlertDialogAction></AlertDialogFooter></AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function LoadingRows() {
  return Array.from({ length: 3 }, (_, index) => (
    <TableRow key={index}>{Array.from({ length: 6 }, (_unused, cell) => <TableCell key={cell}><Skeleton className="h-5 w-24" /></TableCell>)}</TableRow>
  ));
}
