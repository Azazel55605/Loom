import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Check, ChevronLeft, ChevronRight, Loader2, Plus } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
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
import type { Group, User } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import { GroupDialog } from "@loom/ui-kit/pages/settings/GroupsPanel";

const STEPS = ["Account", "Permissions group", "Dashboards", "Summary"] as const;

type KioskSetupWizardProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  groups: Group[];
  groupsUnavailable: boolean;
  onCompleted: () => Promise<void>;
};

/**
 * Guided composition of existing user, group, and dashboard-share APIs.
 *
 * The wizard deliberately submits at the Summary step. If sharing only partly
 * succeeds, it retains the created user and completed dashboard ids while the
 * dialog remains open, so Retry resumes instead of creating a duplicate user.
 */
export function KioskSetupWizard({
  open,
  onOpenChange,
  groups,
  groupsUnavailable,
  onCompleted,
}: KioskSetupWizardProps) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [step, setStep] = React.useState(0);
  const [username, setUsername] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [groupId, setGroupId] = React.useState("");
  const [dashboardIds, setDashboardIds] = React.useState<string[]>([]);
  const [failure, setFailure] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);
  const [createdUser, setCreatedUser] = React.useState<User | null>(null);
  const [sharedDashboardIds, setSharedDashboardIds] = React.useState<string[]>([]);
  const [groupDialogOpen, setGroupDialogOpen] = React.useState(false);

  const dashboards = useQuery({
    queryKey: ["dashboards"],
    queryFn: ({ signal }) => api.getDashboards(signal),
    enabled: open,
    retry: false,
  });
  const permissions = useQuery({
    queryKey: ["permissions"],
    queryFn: ({ signal }) => api.getPermissions(signal),
    enabled: open && groupDialogOpen,
    retry: false,
  });

  const shareableDashboards = React.useMemo(
    () => (dashboards.data ?? []).filter((dashboard) => dashboard.role === "owner"),
    [dashboards.data],
  );

  React.useEffect(() => {
    if (!open) return;
    setStep(0);
    setUsername("");
    setPassword("");
    setGroupId("");
    setDashboardIds([]);
    setFailure(null);
    setSubmitting(false);
    setCreatedUser(null);
    setSharedDashboardIds([]);
    setGroupDialogOpen(false);
  }, [open]);

  function next() {
    setFailure(null);
    if (step === 0) {
      if (username.trim() === "") {
        setFailure("Choose a username.");
        return;
      }
      if (password.length < 8) {
        setFailure("Use at least 8 characters for the password.");
        return;
      }
    }
    if (step === 1 && groupId === "") {
      setFailure("Choose a permissions group.");
      return;
    }
    setStep((current) => Math.min(current + 1, STEPS.length - 1));
  }

  async function finish() {
    setFailure(null);
    setSubmitting(true);
    let user = createdUser;

    try {
      if (user === null) {
        user = await api.createUser({
          username: username.trim(),
          password,
          groupIds: [groupId],
          isKiosk: true,
        });
        setCreatedUser(user);
        // Drop the secret as soon as the account exists. Retrying dashboard
        // shares does not need it, so it should not linger in component state.
        setPassword("");
      }

      const completed = new Set(sharedDashboardIds);
      const errors: string[] = [];
      for (const dashboardId of dashboardIds) {
        if (completed.has(dashboardId)) continue;
        try {
          await api.addDashboardShare(dashboardId, {
            targetType: "user",
            targetId: user.id,
            role: "view",
          });
          completed.add(dashboardId);
          setSharedDashboardIds([...completed]);
        } catch (error: unknown) {
          const dashboard = shareableDashboards.find((entry) => entry.id === dashboardId);
          errors.push(`${dashboard?.name ?? dashboardId}: ${describeAdminFailure(error).message}`);
        }
      }

      if (errors.length > 0) {
        setFailure(`The account was created, but some dashboards were not shared: ${errors.join(" ")}`);
        return;
      }

      await Promise.all([
        onCompleted(),
        queryClient.invalidateQueries({ queryKey: ["dashboards"] }),
      ]);
      toast.success(`Created kiosk user ${user.username}.`);
      onOpenChange(false);
    } catch (error: unknown) {
      setFailure(describeAdminFailure(error).message);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <>
      <Dialog open={open} onOpenChange={(nextOpen) => !submitting && onOpenChange(nextOpen)}>
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>Set up kiosk user</DialogTitle>
            <DialogDescription>
              Create a least-privilege account and give it view access to selected dashboards.
            </DialogDescription>
          </DialogHeader>

          <ol className="grid grid-cols-4 gap-2" aria-label="Kiosk setup progress">
            {STEPS.map((label, index) => (
              <li key={label} className="min-w-0 text-center">
                <div
                  className={`mx-auto mb-1 flex h-7 w-7 items-center justify-center rounded-full border text-xs ${
                    index <= step ? "border-primary bg-primary text-primary-foreground" : "text-muted-foreground"
                  }`}
                >
                  {index < step ? <Check className="h-3.5 w-3.5" aria-hidden="true" /> : index + 1}
                </div>
                <span className="text-xs text-muted-foreground">{label}</span>
              </li>
            ))}
          </ol>

          {failure !== null && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" aria-hidden="true" />
              <AlertTitle>Could not complete kiosk setup</AlertTitle>
              <AlertDescription>{failure}</AlertDescription>
            </Alert>
          )}

          <div className="min-h-64 space-y-4 py-2">
            {step === 0 && (
              <>
                <div className="space-y-2">
                  <Label htmlFor="kiosk-username">Username</Label>
                  <Input
                    id="kiosk-username"
                    autoFocus
                    autoComplete="off"
                    value={username}
                    onChange={(event) => setUsername(event.target.value)}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="kiosk-password">Password</Label>
                  <Input
                    id="kiosk-password"
                    type="password"
                    autoComplete="new-password"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">At least 8 characters.</p>
                </div>
              </>
            )}

            {step === 1 && (
              <>
                <div className="space-y-2">
                  <Label>Permissions group</Label>
                  <Select value={groupId} onValueChange={setGroupId} disabled={groupsUnavailable}>
                    <SelectTrigger>
                      <SelectValue placeholder="Choose a group" />
                    </SelectTrigger>
                    <SelectContent>
                      {groups.map((group) => (
                        <SelectItem key={group.id} value={group.id}>
                          {group.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setGroupDialogOpen(true)}
                  disabled={groupsUnavailable}
                >
                  <Plus aria-hidden="true" />
                  Create new group
                </Button>
                <p className="text-sm text-muted-foreground">
                  Kiosk devices are physically more exposed than a personal login. Consider granting
                  only what the assigned dashboards actually need.
                </p>
              </>
            )}

            {step === 2 && (
              <div className="space-y-3">
                <p className="text-sm text-muted-foreground">
                  Select dashboards you own. Each is shared with this user as a viewer; connector
                  action permissions remain controlled separately by the chosen group.
                </p>
                {dashboards.isPending && <p className="text-sm text-muted-foreground">Loading dashboards…</p>}
                {dashboards.isError && (
                  <p className="text-sm text-destructive">
                    {describeAdminFailure(dashboards.error).message}
                  </p>
                )}
                {dashboards.isSuccess && shareableDashboards.length === 0 && (
                  <p className="rounded-md border p-3 text-sm text-muted-foreground">
                    You do not own any dashboards that can be shared.
                  </p>
                )}
                {shareableDashboards.map((dashboard) => {
                  const checked = dashboardIds.includes(dashboard.id);
                  return (
                    <label
                      key={dashboard.id}
                      className="flex min-h-11 cursor-pointer items-center gap-3 rounded-md border px-3 py-2"
                    >
                      <Checkbox
                        checked={checked}
                        onCheckedChange={(next) =>
                          setDashboardIds((current) =>
                            next === true
                              ? [...current, dashboard.id]
                              : current.filter((id) => id !== dashboard.id),
                          )
                        }
                      />
                      <span className="min-w-0 flex-1 truncate">{dashboard.name}</span>
                      {dashboard.hidden && <Badge variant="secondary">Hidden</Badge>}
                    </label>
                  );
                })}
              </div>
            )}

            {step === 3 && (
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 rounded-md border p-4 text-sm">
                <dt className="text-muted-foreground">Username</dt>
                <dd className="font-medium">{username.trim()}</dd>
                <dt className="text-muted-foreground">Group</dt>
                <dd className="font-medium">
                  {groups.find((group) => group.id === groupId)?.name ?? groupId}
                </dd>
                <dt className="text-muted-foreground">Dashboards</dt>
                <dd className="font-medium">{dashboardIds.length}</dd>
                <dt className="text-muted-foreground">Dashboard role</dt>
                <dd><Badge variant="outline">Viewer</Badge></dd>
              </dl>
            )}
          </div>

          <DialogFooter className="gap-2 sm:justify-between">
            <Button
              type="button"
              variant="outline"
              onClick={() => (step === 0 ? onOpenChange(false) : setStep((current) => current - 1))}
              disabled={submitting || createdUser !== null}
            >
              {step === 0 ? null : <ChevronLeft aria-hidden="true" />}
              {step === 0 ? "Cancel" : "Back"}
            </Button>
            {step < STEPS.length - 1 ? (
              <Button type="button" onClick={next} disabled={submitting}>
                Continue
                <ChevronRight aria-hidden="true" />
              </Button>
            ) : (
              <Button type="button" onClick={finish} disabled={submitting}>
                {submitting && <Loader2 className="animate-spin" aria-hidden="true" />}
                {createdUser === null ? "Create kiosk user" : "Retry dashboard shares"}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <GroupDialog
        open={groupDialogOpen}
        group={null}
        catalog={permissions.data ?? []}
        onOpenChange={setGroupDialogOpen}
        onSaved={async (group) => {
          await queryClient.invalidateQueries({ queryKey: ["groups"] });
          setGroupId(group.id);
        }}
      />
    </>
  );
}
