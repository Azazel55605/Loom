import { Navigate, useLocation, useNavigate, useParams } from "react-router-dom";

import { DesktopAppShell } from "@/components/DesktopAppShell";
import { DashboardsIndexView } from "@loom/ui-kit/components/DashboardsIndexView";
import { DashboardView } from "@loom/ui-kit/components/DashboardView";
import { MotionContent } from "@loom/ui-kit/components/MotionContent";
import {
  dashboardButtonNavigationState,
  readDashboardButtonNavigationState,
} from "@loom/ui-kit/lib/dashboard-navigation";

export function DashboardsIndexPage() {
  const navigate = useNavigate();
  return (
    <DesktopAppShell>
      <DashboardsIndexView
        onNavigate={(dashboardId) => navigate(`/dashboards/${dashboardId}`, { replace: true })}
      />
    </DesktopAppShell>
  );
}

export function DashboardDetailPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const { id } = useParams<{ id: string }>();
  if (id === undefined) return <Navigate to="/dashboards" replace />;
  const buttonNavigation = readDashboardButtonNavigationState(location.state);

  return (
    <DesktopAppShell>
      <MotionContent motionKey={id}>
        <DashboardView
          dashboardId={id}
          onDeleted={() => navigate("/dashboards", { replace: true })}
          // A tile that navigates hands the router the id it was allowed to
          // reach; the UI kit deliberately does not know this app has a router.
          onNavigateDashboard={(target) =>
            navigate(`/dashboards/${target}`, {
              state: dashboardButtonNavigationState(id),
            })
          }
          backNavigation={
            buttonNavigation === null
              ? undefined
              : {
                  fromDashboardId: buttonNavigation.fromDashboardId,
                  onBack: (target) =>
                    navigate(target === null ? "/dashboards" : `/dashboards/${target}`, {
                      replace: true,
                    }),
                }
          }
        />
      </MotionContent>
    </DesktopAppShell>
  );
}
