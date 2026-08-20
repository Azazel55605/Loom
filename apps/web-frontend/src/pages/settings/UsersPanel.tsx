import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { AlertCircle, Loader2, Pencil, Trash2, UserPlus } from "lucide-react";
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
} from "@/components/ui/alert-dialog";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { GroupMultiSelect } from "@/components/GroupMultiSelect";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  createUser,
  deleteUser,
  getGroups,
  getUsers,
  updateUser,
  type Group,
  type User,
} from "@/lib/api";
import { describeAdminFailure } from "@/lib/admin-error";
import { useAuth } from "@/lib/auth-context";

/**
 * User administration.
 *
 * The group list is fetched alongside the users so memberships can be shown by
 * name rather than by id. That fetch needs `groups.manage`, which a user
 * holding only `users.manage` does not have — so it is allowed to fail, and the
 * panel degrades to raw ids with the multi-select disabled rather than
 * presenting the whole page as broken. See `groupsUnavailable` below.
 */
export function UsersPanel() {
  const queryClient = useQueryClient();
  const { user: currentUser } = useAuth();

  const users = useQuery({
    queryKey: ["users"],
    queryFn: ({ signal }) => getUsers(signal),
    retry: false,
  });

  // Not `enabled`-gated on a permission check: the client's view of its own
  // grants is for hiding controls, not for deciding what to request. Asking and
  // handling the 403 keeps the server as the only authority.
  const groups = useQuery({
    queryKey: ["groups"],
    queryFn: ({ signal }) => getGroups(signal),
    retry: false,
  });

  const groupsUnavailable = groups.isError;
  const knownGroups: Group[] = groups.data ?? [];

  const [createOpen, setCreateOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<User | null>(null);
  const [deleting, setDeleting] = React.useState<User | null>(null);

  const invalidate = React.useCallback(async () => {
    // Groups too: membership changes move `memberCount`, which the groups panel
    // shows. Refetching both keeps the two views from disagreeing.
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["users"] }),
      queryClient.invalidateQueries({ queryKey: ["groups"] }),
    ]);
  }, [queryClient]);

  const removeUser = useMutation({
    mutationFn: (target: User) => deleteUser(target.id),
    onSuccess: async (_result, target) => {
      setDeleting(null);
      toast.success(`Deleted ${target.username}.`);
      await invalidate();
    },
    onError: (error: unknown) => {
      const failure = describeAdminFailure(error);
      // Kept open on a refusal: the safeguard message explains a rule about
      // *this* account, and it belongs next to the account it is about.
      toast.error(
        failure.kind === "refused" ? "That deletion was refused" : "Could not delete user",
        { description: failure.message, duration: 10_000 },
      );
    },
  });

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm text-muted-foreground">
          Accounts that can sign in to this instance.
        </p>
        <Button size="sm" onClick={() => setCreateOpen(true)}>
          <UserPlus aria-hidden="true" />
          Add user
        </Button>
      </div>

      {groupsUnavailable && (
        <Alert>
          <AlertCircle className="h-4 w-4" aria-hidden="true" />
          <AlertTitle>Group names unavailable</AlertTitle>
          <AlertDescription>
            Showing group ids instead. Reading the group list needs the{" "}
            <code className="font-mono text-xs">groups.manage</code> permission,
            which this account does not hold — memberships can be viewed but not
            changed.
          </AlertDescription>
        </Alert>
      )}

      {users.isError && (
        <Alert variant="destructive">
          <AlertCircle className="h-4 w-4" aria-hidden="true" />
          <AlertTitle>Could not load users</AlertTitle>
          <AlertDescription>
            {describeAdminFailure(users.error).message}
          </AlertDescription>
        </Alert>
      )}

      <Card>
        <CardContent className="p-2">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-3">Username</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Groups</TableHead>
                <TableHead className="w-24 text-right pr-3">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {users.isPending && <LoadingRows columns={4} />}

              {users.isSuccess && users.data.length === 0 && (
                <TableRow>
                  <TableCell colSpan={4} className="py-6 text-center text-muted-foreground">
                    No accounts.
                  </TableCell>
                </TableRow>
              )}

              {users.data?.map((entry) => {
                const isSelf = entry.id === currentUser?.id;
                return (
                  <TableRow key={entry.id}>
                    <TableCell className="pl-3 font-medium">
                      {entry.username}
                      {isSelf && (
                        <span className="ml-2 text-xs text-muted-foreground">(you)</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge variant={entry.isActive ? "healthy" : "secondary"}>
                        {entry.isActive ? "Active" : "Inactive"}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <GroupBadges groupIds={entry.groupIds} groups={knownGroups} />
                    </TableCell>
                    <TableCell className="pr-3 text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setEditing(entry)}
                          title={`Edit ${entry.username}`}
                        >
                          <Pencil aria-hidden="true" />
                          <span className="sr-only">Edit {entry.username}</span>
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          disabled={isSelf}
                          onClick={() => setDeleting(entry)}
                          title={
                            isSelf
                              ? "You cannot delete your own account."
                              : `Delete ${entry.username}`
                          }
                        >
                          <Trash2 aria-hidden="true" />
                          <span className="sr-only">Delete {entry.username}</span>
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <CreateUserDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        groups={knownGroups}
        groupsUnavailable={groupsUnavailable}
        onCreated={invalidate}
      />

      <EditUserDialog
        user={editing}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        groups={knownGroups}
        groupsUnavailable={groupsUnavailable}
        isSelf={editing?.id === currentUser?.id}
        onUpdated={invalidate}
      />

      <AlertDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {deleting?.username}?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes the account, its group memberships, and its sessions.
              It cannot be undone. To keep the account but stop it signing in,
              edit it and turn off Active instead.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removeUser.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={removeUser.isPending}
              onClick={(event) => {
                // Radix closes on click by default. The dialog has to survive a
                // refusal so the explanation lands somewhere that still exists.
                event.preventDefault();
                if (deleting !== null) removeUser.mutate(deleting);
              }}
            >
              {removeUser.isPending && (
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

/** Membership as one badge per group, by name where the names are known. */
function GroupBadges({ groupIds, groups }: { groupIds: string[]; groups: Group[] }) {
  if (groupIds.length === 0) {
    return <span className="text-sm text-muted-foreground">None</span>;
  }

  return (
    <div className="flex flex-wrap gap-1">
      {groupIds.map((id) => {
        const match = groups.find((group) => group.id === id);
        return (
          <Badge key={id} variant="outline" title={id}>
            {/* Falling back to a truncated id rather than "Unknown": the id is
                the one piece of information actually available, and it is
                enough to match against the groups panel. */}
            {match?.name ?? `${id.slice(0, 8)}…`}
          </Badge>
        );
      })}
    </div>
  );
}

function LoadingRows({ columns }: { columns: number }) {
  return (
    <>
      {[0, 1, 2].map((row) => (
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
/* Create                                                                      */
/* -------------------------------------------------------------------------- */

const createSchema = z.object({
  username: z.string().trim().min(1, "Choose a username."),
  // Eight characters, matching the backend's floor and `SetupPage`'s. A length
  // rule rather than a composition rule, for the reason given there.
  password: z.string().min(8, "Use at least 8 characters."),
  groupIds: z.array(z.string()),
});

type CreateValues = z.infer<typeof createSchema>;

function CreateUserDialog({
  open,
  onOpenChange,
  groups,
  groupsUnavailable,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  groups: Group[];
  groupsUnavailable: boolean;
  onCreated: () => Promise<void>;
}) {
  const [failure, setFailure] = React.useState<string | null>(null);

  const form = useForm<CreateValues>({
    resolver: zodResolver(createSchema),
    defaultValues: { username: "", password: "", groupIds: [] },
  });

  // Reset on open rather than on close, so a reopened dialog is clean even if
  // the previous close came from an Escape mid-typing. Passwords especially
  // should not linger in a form's state across dialogs.
  React.useEffect(() => {
    if (open) {
      form.reset({ username: "", password: "", groupIds: [] });
      setFailure(null);
    }
  }, [open, form]);

  async function onSubmit(values: CreateValues) {
    setFailure(null);
    try {
      await createUser({
        username: values.username.trim(),
        password: values.password,
        groupIds: values.groupIds,
      });
    } catch (error: unknown) {
      setFailure(describeAdminFailure(error).message);
      return;
    }

    toast.success(`Created ${values.username.trim()}.`);
    await onCreated();
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add user</DialogTitle>
          <DialogDescription>
            An account with no groups can sign in and do nothing, which is a
            valid starting point.
          </DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {failure !== null && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" aria-hidden="true" />
                <AlertTitle>Could not create the account</AlertTitle>
                <AlertDescription>{failure}</AlertDescription>
              </Alert>
            )}

            <FormField
              control={form.control}
              name="username"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Username</FormLabel>
                  <FormControl>
                    <Input autoFocus autoComplete="off" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="password"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="new-password" {...field} />
                  </FormControl>
                  <FormDescription>
                    At least 8 characters. Share it with them out of band — Loom
                    cannot show it again.
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="groupIds"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Groups</FormLabel>
                  <FormControl>
                    <GroupMultiSelect
                      groups={groups}
                      value={field.value}
                      onChange={field.onChange}
                      disabled={groupsUnavailable}
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
                Create user
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

/* -------------------------------------------------------------------------- */
/* Edit                                                                        */
/* -------------------------------------------------------------------------- */

const editSchema = z.object({
  isActive: z.boolean(),
  groupIds: z.array(z.string()),
});

type EditValues = z.infer<typeof editSchema>;

function EditUserDialog({
  user,
  onOpenChange,
  groups,
  groupsUnavailable,
  isSelf,
  onUpdated,
}: {
  user: User | null;
  onOpenChange: (open: boolean) => void;
  groups: Group[];
  groupsUnavailable: boolean;
  isSelf: boolean;
  onUpdated: () => Promise<void>;
}) {
  const [failure, setFailure] = React.useState<string | null>(null);

  const form = useForm<EditValues>({
    resolver: zodResolver(editSchema),
    defaultValues: { isActive: true, groupIds: [] },
  });

  // Refill whenever a different user is opened. The dialog is one instance
  // reused for every row, so without this it would show the previous row's
  // state for a frame — and worse, submit it if the user was quick.
  React.useEffect(() => {
    if (user !== null) {
      form.reset({ isActive: user.isActive, groupIds: user.groupIds });
      setFailure(null);
    }
  }, [user, form]);

  async function onSubmit(values: EditValues) {
    if (user === null) return;
    setFailure(null);
    try {
      await updateUser(user.id, {
        isActive: values.isActive,
        groupIds: values.groupIds,
      });
    } catch (error: unknown) {
      setFailure(describeAdminFailure(error).message);
      return;
    }

    toast.success(`Updated ${user.username}.`);
    await onUpdated();
    onOpenChange(false);
  }

  return (
    <Dialog open={user !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit {user?.username}</DialogTitle>
          <DialogDescription>
            Group membership is replaced with exactly what is ticked here.
          </DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {failure !== null && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" aria-hidden="true" />
                <AlertTitle>Could not apply the change</AlertTitle>
                <AlertDescription>{failure}</AlertDescription>
              </Alert>
            )}

            <FormField
              control={form.control}
              name="isActive"
              render={({ field }) => (
                <FormItem className="flex items-center justify-between gap-4 rounded-md border p-3">
                  <div className="space-y-0.5">
                    <FormLabel>Active</FormLabel>
                    <FormDescription>
                      {isSelf
                        ? "You cannot deactivate your own account."
                        : "An inactive account cannot sign in. Existing sessions end at their next refresh."}
                    </FormDescription>
                  </div>
                  <FormControl>
                    <Switch
                      checked={field.value}
                      onCheckedChange={field.onChange}
                      // The backend refuses this specific change for the caller's
                      // own account. Disabling it is the same rule stated where
                      // the user is, rather than after they try.
                      disabled={isSelf}
                      aria-label="Active"
                    />
                  </FormControl>
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="groupIds"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Groups</FormLabel>
                  <FormControl>
                    <GroupMultiSelect
                      groups={groups}
                      value={field.value}
                      onChange={field.onChange}
                      disabled={groupsUnavailable}
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
                Save changes
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
