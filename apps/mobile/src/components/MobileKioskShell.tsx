import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LockKeyhole } from "lucide-react";

import { KioskExitDialog } from "@/components/KioskExitDialog";
import { dashboardsQueryKey } from "@loom/ui-kit/components/DashboardSidebar";
import { DashboardView } from "@loom/ui-kit/components/DashboardView";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

const SWIPE_THRESHOLD_PX = 60;
const EXIT_HOLD_MS = 3_000;

export function MobileKioskShell({ onExited }: { onExited: () => void }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
  });
  const [index, setIndex] = React.useState(0);
  const [exitOpen, setExitOpen] = React.useState(false);
  const touchStart = React.useRef<{ x: number; y: number } | null>(null);
  const holdTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const dashboardList = dashboards.data ?? [];
  const boundedIndex = Math.min(index, Math.max(dashboardList.length - 1, 0));
  const activeDashboard = dashboardList[boundedIndex];

  React.useEffect(() => {
    if (index !== boundedIndex) setIndex(boundedIndex);
  }, [boundedIndex, index]);

  React.useEffect(
    () => () => {
      if (holdTimer.current !== null) clearTimeout(holdTimer.current);
    },
    [],
  );

  function stopExitHold() {
    if (holdTimer.current !== null) clearTimeout(holdTimer.current);
    holdTimer.current = null;
  }

  function navigateToDashboard(dashboardId: string) {
    const nextIndex = dashboardList.findIndex((dashboard) => dashboard.id === dashboardId);
    if (nextIndex >= 0) setIndex(nextIndex);
  }

  return (
    <main
      className="mobile-kiosk-shell app-canvas"
      onTouchStart={(event) => {
        const touch = event.changedTouches[0];
        touchStart.current = { x: touch.clientX, y: touch.clientY };
      }}
      onTouchEnd={(event) => {
        const start = touchStart.current;
        touchStart.current = null;
        if (start === null || dashboardList.length < 2) return;
        const touch = event.changedTouches[0];
        const deltaX = touch.clientX - start.x;
        const deltaY = touch.clientY - start.y;
        if (Math.abs(deltaX) < SWIPE_THRESHOLD_PX || Math.abs(deltaX) <= Math.abs(deltaY)) return;
        setIndex((current) =>
          deltaX < 0
            ? Math.min(current + 1, dashboardList.length - 1)
            : Math.max(current - 1, 0),
        );
      }}
    >
      <div className="min-h-full p-3 sm:p-5">
        {dashboards.isPending ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-8 w-48" />
            <Skeleton className="h-64 w-full" />
          </div>
        ) : null}
        {dashboards.isError ? (
          <Alert variant="destructive">
            <AlertTitle>Could not load kiosk dashboards</AlertTitle>
            <AlertDescription>{describeConnectorError(dashboards.error)}</AlertDescription>
          </Alert>
        ) : null}
        {dashboards.isSuccess && activeDashboard === undefined ? (
          <div className="flex min-h-[70dvh] items-center justify-center text-center text-sm text-muted-foreground">
            No dashboards are assigned to this kiosk account.
          </div>
        ) : null}
        {activeDashboard !== undefined ? (
          <DashboardView
            key={activeDashboard.id}
            dashboardId={activeDashboard.id}
            onDeleted={() => undefined}
            onNavigateDashboard={navigateToDashboard}
          />
        ) : null}
      </div>

      {dashboardList.length > 1 ? (
        <div
          className="mobile-kiosk-dots surface-elevated"
          aria-label={`Dashboard ${boundedIndex + 1} of ${dashboardList.length}`}
        >
          {dashboardList.map((dashboard, dashboardIndex) => (
            <Button
              key={dashboard.id}
              type="button"
              variant="ghost"
              size="icon"
              className="mobile-kiosk-dot-hit"
              aria-label={`Show ${dashboard.name}`}
              aria-current={dashboardIndex === boundedIndex ? "page" : undefined}
              onClick={() => setIndex(dashboardIndex)}
            >
              <span
                className={cn("mobile-kiosk-dot", dashboardIndex === boundedIndex && "bg-primary")}
                aria-hidden="true"
              />
            </Button>
          ))}
        </div>
      ) : null}

      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="mobile-kiosk-exit-hold"
        aria-label="Hold for three seconds to exit kiosk mode"
        onContextMenu={(event) => event.preventDefault()}
        onPointerDown={() => {
          stopExitHold();
          holdTimer.current = setTimeout(() => {
            holdTimer.current = null;
            setExitOpen(true);
          }, EXIT_HOLD_MS);
        }}
        onPointerUp={stopExitHold}
        onPointerCancel={stopExitHold}
        onPointerLeave={stopExitHold}
      >
        <LockKeyhole aria-hidden="true" />
      </Button>

      {user !== null ? (
        <KioskExitDialog
          open={exitOpen}
          activeKioskUserId={user.id}
          onOpenChange={setExitOpen}
          onExit={() => {
            // The next identity must not inherit kiosk-owned cached dashboard
            // or connector responses while its own queries refetch.
            queryClient.clear();
            onExited();
          }}
        />
      ) : null}
    </main>
  );
}
