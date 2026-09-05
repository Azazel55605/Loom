import { Navigate, useNavigate, useParams } from "react-router-dom";

import { WebAppShell } from "@/components/WebAppShell";
import { DashboardsIndexView } from "@loom/ui-kit/components/DashboardsIndexView";
import { DashboardView } from "@loom/ui-kit/components/DashboardView";

export function DashboardsIndexPage() {
  const navigate = useNavigate();

  return (
    <WebAppShell>
      <DashboardsIndexView
        onNavigate={(dashboardId) => navigate(`/dashboards/${dashboardId}`, { replace: true })}
      />
    </WebAppShell>
  );
}

export function DashboardDetailPage() {
  const navigate = useNavigate();
  const { id } = useParams<{ id: string }>();
  if (id === undefined) return <Navigate to="/dashboards" replace />;

  return (
    <WebAppShell>
      <DashboardView
        key={id}
        dashboardId={id}
        onDeleted={() => navigate("/dashboards", { replace: true })}
        // A tile that navigates hands the router the id it was allowed to
        // reach; the UI kit deliberately does not know this app has a router.
        onNavigateDashboard={(target) => navigate(`/dashboards/${target}`)}
      />
    </WebAppShell>
  );
}
