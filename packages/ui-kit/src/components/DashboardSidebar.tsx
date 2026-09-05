import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, ChevronDown, Pin, PinOff, Plus } from "lucide-react";

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
  DialogTrigger,
} from "@loom/ui-kit/components/ui/dialog";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import type { DashboardSummary } from "@loom/ui-kit/lib/api";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

export const dashboardsQueryKey = ["dashboards"] as const;

/** Dashboard navigation shared by every client; the host supplies routing. */
export function DashboardSidebar({
  activeDashboardId,
  onNavigate,
  footerControl,
}: {
  activeDashboardId?: string;
  onNavigate: (dashboardId: string) => void;
  /** Optional platform navigation for adjacent management surfaces. */
  footerControl?: React.ReactNode;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
  });

  const changePin = useMutation({
    mutationFn: ({ dashboard, pinned }: { dashboard: DashboardSummary; pinned: boolean }) =>
      pinned ? api.pinDashboard(dashboard.id) : api.unpinDashboard(dashboard.id),
    onMutate: async ({ dashboard, pinned }) => {
      await queryClient.cancelQueries({ queryKey: dashboardsQueryKey });
      const previous = queryClient.getQueryData<DashboardSummary[]>(dashboardsQueryKey);
      queryClient.setQueryData<DashboardSummary[]>(dashboardsQueryKey, (current) =>
        current?.map((item) =>
          item.id === dashboard.id ? { ...item, pinned } : item,
        ),
      );
      return { previous };
    },
    onError: (_error, _variables, context) => {
      queryClient.setQueryData(dashboardsQueryKey, context?.previous);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: dashboardsQueryKey });
    },
  });

  /**
   * Hidden dashboards are left out of **every** section, including Pinned.
   *
   * The flag means "do not offer this in a list", and pinning or owning one
   * does not make it less hidden — a dashboard that reappeared under Pinned
   * would make the setting look broken to the person who set it. It stays
   * reachable by id and through any button tile that navigates to it, which is
   * what hiding is normally for; its owner unhides it from the dashboard's own
   * header, which they reach the same way.
   *
   * The filter lives here rather than in `GET /dashboards`, which keeps
   * returning them: a client that suppressed them server-side would have no way
   * to show one, and no way to turn the flag back off.
   */
  const visible = dashboards.data?.filter((dashboard) => !dashboard.hidden) ?? [];
  const pinned = visible.filter((dashboard) => dashboard.pinned);
  const owned = visible.filter((dashboard) => dashboard.role === "owner");
  const shared = visible.filter((dashboard) => dashboard.role !== "owner");

  return (
    <nav aria-label="Dashboard navigation" className="flex h-full flex-col gap-4 p-4">
      <DashboardCreateDialog
        onCreated={(dashboard) => onNavigate(dashboard.id)}
        trigger={
          <Button className="w-full" size="sm">
            <Plus aria-hidden="true" />
            New dashboard
          </Button>
        }
      />

      {dashboards.isPending ? <DashboardSidebarSkeleton /> : null}

      {dashboards.isError ? (
        <Alert variant="destructive" className="px-3 py-2">
          <AlertCircle aria-hidden="true" />
          <AlertDescription>{describeConnectorError(dashboards.error)}</AlertDescription>
        </Alert>
      ) : null}

      {dashboards.isSuccess ? (
        <div className="flex flex-col gap-2">
          {pinned.length > 0 ? (
            <DashboardSection title="Pinned">
              {pinned.map((dashboard) => (
                <DashboardSidebarItem
                  key={dashboard.id}
                  dashboard={dashboard}
                  active={dashboard.id === activeDashboardId}
                  pinPending={
                    changePin.isPending && changePin.variables.dashboard.id === dashboard.id
                  }
                  onNavigate={onNavigate}
                  onTogglePin={() => changePin.mutate({ dashboard, pinned: false })}
                />
              ))}
            </DashboardSection>
          ) : null}

          <DashboardSection title="My Dashboards">
            {owned.length > 0 ? (
              owned.map((dashboard) => (
                <DashboardSidebarItem
                  key={dashboard.id}
                  dashboard={dashboard}
                  active={dashboard.id === activeDashboardId}
                  pinPending={
                    changePin.isPending && changePin.variables.dashboard.id === dashboard.id
                  }
                  onNavigate={onNavigate}
                  onTogglePin={() =>
                    changePin.mutate({ dashboard, pinned: !dashboard.pinned })
                  }
                />
              ))
            ) : (
              <p className="px-2 py-1 text-xs text-muted-foreground">
                Create a dashboard to arrange your services.
              </p>
            )}
          </DashboardSection>

          {shared.length > 0 ? (
            <DashboardSection title="Shared with me">
              {shared.map((dashboard) => (
                <DashboardSidebarItem
                  key={dashboard.id}
                  dashboard={dashboard}
                  active={dashboard.id === activeDashboardId}
                  pinPending={
                    changePin.isPending && changePin.variables.dashboard.id === dashboard.id
                  }
                  onNavigate={onNavigate}
                  onTogglePin={() =>
                    changePin.mutate({ dashboard, pinned: !dashboard.pinned })
                  }
                />
              ))}
            </DashboardSection>
          ) : null}
        </div>
      ) : null}

      {footerControl === undefined ? null : (
        <div className="mt-auto border-t pt-4">{footerControl}</div>
      )}
    </nav>
  );
}

/** Reusable create transaction for the sidebar and first-dashboard empty state. */
export function DashboardCreateDialog({
  trigger,
  onCreated,
}: {
  trigger: React.ReactNode;
  onCreated: (dashboard: DashboardSummary) => void;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [open, setOpen] = React.useState(false);
  const [name, setName] = React.useState("");

  const createDashboard = useMutation({
    mutationFn: () => api.createDashboard(name),
    onSuccess: async (dashboard) => {
      queryClient.setQueryData<DashboardSummary[]>(dashboardsQueryKey, (current) =>
        current === undefined ? [dashboard] : [...current, dashboard],
      );
      await queryClient.invalidateQueries({ queryKey: dashboardsQueryKey });
      setOpen(false);
      setName("");
      onCreated(dashboard);
    },
  });

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          createDashboard.reset();
          setName("");
        }
      }}
    >
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New dashboard</DialogTitle>
          <DialogDescription>
            Create a dashboard now; connector placement is added in the next update.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (name.trim()) createDashboard.mutate();
          }}
        >
          <div className="flex flex-col gap-2">
            <Label htmlFor="new-dashboard-name">Name</Label>
            <Input
              id="new-dashboard-name"
              autoFocus
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Operations"
            />
          </div>
          {createDashboard.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertDescription>
                {describeConnectorError(createDashboard.error)}
              </AlertDescription>
            </Alert>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!name.trim() || createDashboard.isPending}>
              {createDashboard.isPending ? "Creating…" : "Create dashboard"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DashboardSection({ title, children }: { title: string; children: React.ReactNode }) {
  const [open, setOpen] = React.useState(true);
  return (
    <section>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="w-full justify-between px-2 text-muted-foreground"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        {title}
        <ChevronDown
          aria-hidden="true"
          className={cn("transition-transform", !open && "-rotate-90")}
        />
      </Button>
      {open ? <div className="mt-1 flex flex-col gap-1">{children}</div> : null}
    </section>
  );
}

function DashboardSidebarItem({
  dashboard,
  active,
  pinPending,
  onNavigate,
  onTogglePin,
}: {
  dashboard: DashboardSummary;
  active: boolean;
  pinPending: boolean;
  onNavigate: (dashboardId: string) => void;
  onTogglePin: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex min-w-0 items-center rounded-md",
        active && "bg-muted text-foreground",
      )}
    >
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="min-w-0 flex-1 justify-start px-2"
        aria-current={active ? "page" : undefined}
        onClick={() => onNavigate(dashboard.id)}
      >
        <span className="truncate">{dashboard.name}</span>
        {dashboard.role !== "owner" ? (
          <Badge variant="outline" className="ml-auto px-1.5 py-0 text-[10px]">
            {dashboard.role === "editor" ? "Editor" : "Viewer"}
          </Badge>
        ) : null}
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-8 w-8 shrink-0"
        disabled={pinPending}
        aria-label={`${dashboard.pinned ? "Unpin" : "Pin"} ${dashboard.name}`}
        onClick={onTogglePin}
      >
        {dashboard.pinned ? <PinOff aria-hidden="true" /> : <Pin aria-hidden="true" />}
      </Button>
    </div>
  );
}

function DashboardSidebarSkeleton() {
  return (
    <div className="flex flex-col gap-3" aria-label="Loading dashboards">
      <Skeleton className="h-8 w-full" />
      <Skeleton className="h-8 w-5/6" />
      <Skeleton className="h-8 w-full" />
    </div>
  );
}
