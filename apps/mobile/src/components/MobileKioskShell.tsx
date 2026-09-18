import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LockKeyhole, MoonStar } from "lucide-react";

import { KioskExitDialog } from "@/components/KioskExitDialog";
import { dashboardsQueryKey } from "@loom/ui-kit/components/DashboardSidebar";
import { DashboardView, dashboardQueryKey } from "@loom/ui-kit/components/DashboardView";
import { MotionContent } from "@loom/ui-kit/components/MotionContent";
import { NetworkAdvisoryBanner } from "@loom/ui-kit/components/NetworkAdvisoryBanner";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { ScreensaverView } from "@/components/ScreensaverView";
import { useMobileKioskMode } from "@/components/mobileKioskMode";

const SWIPE_THRESHOLD_PX = 60;
const EXIT_HOLD_MS = 3_000;
/** Long enough that swiping through the rotation does not refetch each step. */
const PREFETCH_STALE_MS = 60_000;

export function MobileKioskShell({ onExited }: { onExited: () => void }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const kiosk = useMobileKioskMode();
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
  });
  const [index, setIndex] = React.useState(0);
  const [offRotationId, setOffRotationId] = React.useState<string | null>(null);
  const [exitOpen, setExitOpen] = React.useState(false);
  const [screensaverVisible, setScreensaverVisible] = React.useState(false);
  const lastActivityAt = React.useRef(Date.now());
  const touchStart = React.useRef<{ x: number; y: number } | null>(null);
  const holdTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const allDashboards = React.useMemo(() => dashboards.data ?? [], [dashboards.data]);
  // Hidden dashboards stay out of the swipeable rotation exactly as they stay
  // out of `DashboardSidebar`'s listings. They remain reachable by id, which is
  // what a `navigate` tile pointing at one needs — see ADR 0035.
  const dashboardList = React.useMemo(
    () => allDashboards.filter((dashboard) => !dashboard.hidden),
    [allDashboards],
  );
  const boundedIndex = Math.min(index, Math.max(dashboardList.length - 1, 0));
  const offRotationDashboard =
    offRotationId === null
      ? undefined
      : allDashboards.find((dashboard) => dashboard.id === offRotationId);
  const activeDashboard = offRotationDashboard ?? dashboardList[boundedIndex];

  React.useEffect(() => {
    if (index !== boundedIndex) setIndex(boundedIndex);
  }, [boundedIndex, index]);

  // Warm the dashboards a swipe can reach next.
  //
  // Only the *structure* query is prefetched, and deliberately so: filling this
  // cache key removes the loading skeleton on arrival, while opening a
  // connector status subscription for a dashboard nobody is looking at would
  // put many instances on the socket for no visible benefit. Live status stays
  // the business of the mounted `DashboardView`, which subscribes when a
  // dashboard actually becomes visible.
  //
  // It waits for the visible dashboard to settle first — `ensureQueryData`
  // resolves immediately when it is already cached — so a prefetch never
  // competes with the fetch the person is waiting on.
  React.useEffect(() => {
    if (activeDashboard === undefined || dashboardList.length < 2) return;
    let cancelled = false;

    void (async () => {
      try {
        await queryClient.ensureQueryData({
          queryKey: dashboardQueryKey(activeDashboard.id),
          queryFn: ({ signal }) => api.getDashboard(activeDashboard.id, signal),
        });
      } catch {
        // The visible view owns reporting its own failure.
        return;
      }
      if (cancelled) return;

      const neighbours = new Set<string>();
      const current = dashboardList.findIndex(
        (dashboard) => dashboard.id === activeDashboard.id,
      );
      if (current >= 0) {
        const { length } = dashboardList;
        // Wrapped, matching the swipe: at either end the neighbour is the
        // dashboard on the other side of the rotation.
        neighbours.add(dashboardList[(current + 1) % length].id);
        neighbours.add(dashboardList[(current - 1 + length) % length].id);
      }
      neighbours.delete(activeDashboard.id);

      for (const dashboardId of neighbours) {
        void queryClient.prefetchQuery({
          queryKey: dashboardQueryKey(dashboardId),
          queryFn: ({ signal }) => api.getDashboard(dashboardId, signal),
          staleTime: PREFETCH_STALE_MS,
        });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeDashboard, api, dashboardList, queryClient]);

  React.useEffect(
    () => () => {
      if (holdTimer.current !== null) clearTimeout(holdTimer.current);
    },
    [],
  );

  React.useEffect(() => {
    if (!kiosk.screensaverEnabled) {
      setScreensaverVisible(false);
      return;
    }
    const recordActivity = () => {
      lastActivityAt.current = Date.now();
      setScreensaverVisible(false);
    };
    const checkIdle = () => {
      if (Date.now() - lastActivityAt.current >= kiosk.screensaverIdleSeconds * 1_000) {
        setScreensaverVisible(true);
      }
    };
    window.addEventListener("pointerdown", recordActivity, { passive: true });
    window.addEventListener("touchstart", recordActivity, { passive: true });
    const timer = window.setInterval(checkIdle, 1_000);
    return () => {
      window.removeEventListener("pointerdown", recordActivity);
      window.removeEventListener("touchstart", recordActivity);
      window.clearInterval(timer);
    };
  }, [kiosk.screensaverEnabled, kiosk.screensaverIdleSeconds]);

  const account = useQuery({
    queryKey: ["account"],
    queryFn: ({ signal }) => api.getAccount(signal),
  });

  function stopExitHold() {
    if (holdTimer.current !== null) clearTimeout(holdTimer.current);
    holdTimer.current = null;
  }

  function navigateToDashboard(dashboardId: string) {
    const nextIndex = dashboardList.findIndex((dashboard) => dashboard.id === dashboardId);
    if (nextIndex >= 0) {
      setOffRotationId(null);
      setIndex(nextIndex);
      return;
    }
    // A tile may target a hidden dashboard, which has no rotation slot. Show it
    // outside the rotation; the next swipe or dot tap returns to the rotation.
    if (allDashboards.some((dashboard) => dashboard.id === dashboardId)) {
      setOffRotationId(dashboardId);
    }
  }

  function showRotationIndex(nextIndex: number) {
    setOffRotationId(null);
    setIndex(nextIndex);
  }

  function sleepNow() {
    // The idle watcher treats any pointer event as activity, including the tap
    // that lands here, so the timestamp is moved along with the state: without
    // it the watcher would immediately see fresh activity and wake the display
    // back up on the next tick.
    lastActivityAt.current = Date.now();
    setScreensaverVisible(true);
  }

  if (screensaverVisible) {
    return (
      <ScreensaverView
        config={account.data?.screensaverConfig ?? []}
        onDismiss={() => {
          lastActivityAt.current = Date.now();
          setScreensaverVisible(false);
        }}
      />
    );
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
        const rotationLength = dashboardList.length;
        if (start === null || rotationLength === 0) return;
        if (rotationLength < 2 && offRotationId === null) return;
        const touch = event.changedTouches[0];
        const deltaX = touch.clientX - start.x;
        const deltaY = touch.clientY - start.y;
        if (Math.abs(deltaX) < SWIPE_THRESHOLD_PX || Math.abs(deltaX) <= Math.abs(deltaY)) return;
        const step = deltaX < 0 ? 1 : -1;
        setOffRotationId(null);
        // The rotation wraps: past the last dashboard is the first, and before
        // the first is the last.
        setIndex((current) => (current + step + rotationLength) % rotationLength);
      }}
    >
      <NetworkAdvisoryBanner />
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
            No visible dashboards are assigned to this kiosk account.
          </div>
        ) : null}
        {activeDashboard !== undefined ? (
          <MotionContent motionKey={activeDashboard.id}>
            <DashboardView
              dashboardId={activeDashboard.id}
              onDeleted={() => undefined}
              onNavigateDashboard={navigateToDashboard}
            />
          </MotionContent>
        ) : null}
      </div>

      {dashboardList.length > 1 ? (
        <div
          className="mobile-kiosk-dots surface-elevated"
          aria-label={
            offRotationDashboard === undefined
              ? `Dashboard ${boundedIndex + 1} of ${dashboardList.length}`
              : `Showing ${offRotationDashboard.name}, which is not part of the rotation`
          }
        >
          {dashboardList.map((dashboard, dashboardIndex) => (
            <Button
              key={dashboard.id}
              type="button"
              variant="ghost"
              size="icon"
              className="mobile-kiosk-dot-hit"
              aria-label={`Show ${dashboard.name}`}
              aria-current={
                offRotationDashboard === undefined && dashboardIndex === boundedIndex
                  ? "page"
                  : undefined
              }
              onClick={() => showRotationIndex(dashboardIndex)}
            >
              <span
                className="mobile-kiosk-dot"
                data-active={
                  offRotationDashboard === undefined && dashboardIndex === boundedIndex
                    ? "true"
                    : "false"
                }
                aria-hidden="true"
              />
            </Button>
          ))}
          {activeDashboard !== undefined ? (
            <span className="mobile-kiosk-dot-label">{activeDashboard.name}</span>
          ) : null}
        </div>
      ) : null}

      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="mobile-kiosk-sleep"
        aria-label="Sleep this display now"
        onClick={sleepNow}
      >
        <MoonStar aria-hidden="true" />
      </Button>

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
