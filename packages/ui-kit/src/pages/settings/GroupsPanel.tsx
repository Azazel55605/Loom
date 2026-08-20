import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { AlertCircle, Loader2, Pencil, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { z } from "zod";

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
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@loom/ui-kit/components/ui/form";
import { Input } from "@loom/ui-kit/components/ui/input";
import { PermissionGrantBuilder } from "@loom/ui-kit/components/PermissionGrantBuilder";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@loom/ui-kit/components/ui/table";
import type { Group, PermissionGrant } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";

/**
 * Group administration: names, descriptions, and permission grants.
 *
 * The protected group is the one an instance cannot be administered without.
 * Its delete control is not rendered at all rather than rendered-and-refused —
 * the contract asks clients to do this, and a control that exists only to
 * produce a 409 teaches people that errors are normal.
 */
export function GroupsPanel() {
  const api = useApiClient();
  const queryClient = useQueryClient();

  const groups = useQuery({
    queryKey: ["groups"],
    queryFn: ({ signal }) => api.getGroups(signal),
    retry: false,
  });

  const permissions = useQuery({
    queryKey: ["permissions"],
    queryFn: ({ signal }) => api.getPermissions(signal),
    retry: false,
    // The catalog changes only when a migration registers a key, which means a
    // redeploy. It does not need refetching within a session.
    staleTime: Infinity,
  });

  const [createOpen, setCreateOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<Group | null>(null);
  const [deleting, setDeleting] = React.useState<Group | null>(null);

  const invalidate = React.useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ["groups"] });
  }, [queryClient]);

  const removeGroup = useMutation({
    mutationFn: (target: Group) => api.deleteGroup(target.id),
    onSuccess: async (_result, target) => {
      setDeleting(null);
      toast.success(`Deleted ${target.name}.`);
      // Memberships go with the group, so any user row showing it is now stale.
      await Promise.all([
        invalidate(),
        queryClient.invalidateQueries({ queryKey: ["users"] }),
      ]);
    },
    onError: (error: unknown) => {
      const failure = describeAdminFailure(error);
      toast.error(
        failure.kind === "refused" ? "That deletion was refused" : "Could not delete group",
        { description: failure.message, duration: 10_000 },
      );
    },
  });

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm text-muted-foreground">
          Groups carry permissions. Users get their access by belonging to one.
        </p>
        <Button size="sm" onClick={() => setCreateOpen(true)}>
          <Plus aria-hidden="true" />
          Add group
        </Button>
      </div>

      {groups.isError && (
        <Alert variant="destructive">
          <AlertCircle className="h-4 w-4" aria-hidden="true" />
          <AlertTitle>Could not load groups</AlertTitle>
          <AlertDescription>
            {describeAdminFailure(groups.error).message}
          </AlertDescription>
        </Alert>
      )}

      <Card>
        <CardContent className="p-2">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-3">Name</TableHead>
                <TableHead>Description</TableHead>
                <TableHead className="w-24">Members</TableHead>
                <TableHead className="w-24 pr-3 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {groups.isPending && <LoadingRows columns={4} />}

              {groups.isSuccess && groups.data.length === 0 && (
                <TableRow>
                  <TableCell colSpan={4} className="py-6 text-center text-muted-foreground">
                    No groups.
                  </TableCell>
                </TableRow>
              )}

              {groups.data?.map((group) => (
                <TableRow key={group.id}>
                  <TableCell className="pl-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{group.name}</span>
                      {group.isProtected && (
                        <Badge
                          variant="secondary"
                          title="This group cannot be deleted — an instance without it cannot be administered."
                        >
                          Protected
                        </Badge>
                      )}
                    </div>
                    <PermissionSummary permissions={group.permissions} />
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {group.description ?? "—"}
                  </TableCell>
                  <TableCell>{group.memberCount}</TableCell>
                  <TableCell className="pr-3 text-right">
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => setEditing(group)}
                        title={`Edit ${group.name}`}
                      >
                        <Pencil aria-hidden="true" />
                        <span className="sr-only">Edit {group.name}</span>
                      </Button>
                      {/* Not rendered at all for a protected group. A disabled
                          button would still invite the question; its absence
                          matches the fact that deletion is not on offer. */}
                      {!group.isProtected && (
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setDeleting(group)}
                          title={`Delete ${group.name}`}
                        >
                          <Trash2 aria-hidden="true" />
                          <span className="sr-only">Delete {group.name}</span>
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <GroupDialog
        key={editing?.id ?? "create"}
        open={createOpen || editing !== null}
        group={editing}
        catalog={permissions.data ?? []}
        onOpenChange={(open) => {
          if (!open) {
            setCreateOpen(false);
            setEditing(null);
          }
        }}
        onSaved={invalidate}
      />

      <AlertDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {deleting?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              {deleting?.memberCount === 1
                ? "One member loses everything this group granted them. "
                : `${deleting?.memberCount ?? 0} members lose everything this group granted them. `}
              Their accounts are not affected. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removeGroup.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={removeGroup.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (deleting !== null) removeGroup.mutate(deleting);
              }}
            >
              {removeGroup.isPending && (
                <Loader2 className="animate-spin" aria-hidden="true" />
              )}
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** The group's grants, in one line, scope included. */
function PermissionSummary({ permissions }: { permissions: PermissionGrant[] }) {
  if (permissions.length === 0) {
    return <p className="mt-1 text-xs text-muted-foreground">No permissions.</p>;
  }

  return (
    <p className="mt-1 font-mono text-xs text-muted-foreground">
      {permissions
        .map((grant) =>
          grant.resourceId === null ? grant.key : `${grant.key} → ${grant.resourceId}`,
        )
        .join(", ")}
    </p>
  );
}

function LoadingRows({ columns }: { columns: number }) {
  return (
    <>
      {[0, 1].map((row) => (
        <TableRow key={row}>
          {Array.from({ length: columns }, (_, column) => (
            <TableCell key={column}>
              <Skeleton className="h-4 w-24" />
            </TableCell>
          ))}
        </TableRow>
      ))}
    </>
  );
}

/* -------------------------------------------------------------------------- */
/* Create and edit                                                             */
/* -------------------------------------------------------------------------- */

const groupSchema = z.object({
  name: z.string().trim().min(1, "Give the group a name."),
  description: z.string(),
  permissions: z.array(
    z.object({
      key: z.string(),
      resourceType: z.string().nullable(),
      resourceId: z.string().nullable(),
    }),
  ),
});

type GroupValues = z.infer<typeof groupSchema>;

/**
 * One dialog for both creating and editing.
 *
 * The two differ only in which request is sent: the fields, the validation, and
 * the grant builder are identical, because `PATCH` states the whole grant set
 * exactly as `POST` does. Two dialogs would be one form duplicated, and the
 * copy that gets edited less is the one that drifts.
 */
function GroupDialog({
  open,
  group,
  catalog,
  onOpenChange,
  onSaved,
}: {
  open: boolean;
  /** The group being edited, or null to create a new one. */
  group: Group | null;
  catalog: React.ComponentProps<typeof PermissionGrantBuilder>["catalog"];
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<void>;
}) {
  const api = useApiClient();
  const [failure, setFailure] = React.useState<string | null>(null);

  const form = useForm<GroupValues>({
    resolver: zodResolver(groupSchema),
    defaultValues: {
      name: group?.name ?? "",
      // The API distinguishes null from empty; the form field cannot hold null,
      // so it round-trips through "" and is mapped back on submit.
      description: group?.description ?? "",
      permissions: group?.permissions ?? [],
    },
  });

  React.useEffect(() => {
    if (open) setFailure(null);
  }, [open]);

  async function onSubmit(values: GroupValues) {
    setFailure(null);
    const description = values.description.trim();

    try {
      if (group === null) {
        await api.createGroup({
          name: values.name.trim(),
          description: description === "" ? null : description,
          permissions: values.permissions,
        });
      } else {
        await api.updateGroup(group.id, {
          name: values.name.trim(),
          description: description === "" ? null : description,
          permissions: values.permissions,
        });
      }
    } catch (error: unknown) {
      setFailure(describeAdminFailure(error).message);
      return;
    }

    toast.success(group === null ? `Created ${values.name.trim()}.` : `Updated ${values.name.trim()}.`);
    await onSaved();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{group === null ? "Add group" : `Edit ${group.name}`}</DialogTitle>
          <DialogDescription>
            {group?.isProtected === true
              ? "This group is protected: it can be renamed and re-granted, but not deleted."
              : "Permissions are replaced with exactly what is ticked here."}
          </DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {failure !== null && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" aria-hidden="true" />
                <AlertTitle>Could not save the group</AlertTitle>
                <AlertDescription>{failure}</AlertDescription>
              </Alert>
            )}

            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Name</FormLabel>
                  <FormControl>
                    <Input autoFocus placeholder="Viewers" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <Input placeholder="Read-only access." {...field} />
                  </FormControl>
                  <FormDescription>Optional.</FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="permissions"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Permissions</FormLabel>
                  <FormControl>
                    <PermissionGrantBuilder
                      catalog={catalog}
                      value={field.value}
                      onChange={field.onChange}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
                disabled={form.formState.isSubmitting}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={form.formState.isSubmitting}>
                {form.formState.isSubmitting && (
                  <Loader2 className="animate-spin" aria-hidden="true" />
                )}
                {group === null ? "Create group" : "Save changes"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
