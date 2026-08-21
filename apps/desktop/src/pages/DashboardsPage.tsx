import { Navigate, useNavigate, useParams } from "react-router-dom";

import { DesktopAppShell } from "@/components/DesktopAppShell";
import { DashboardsIndexView } from "@loom/ui-kit/components/DashboardsIndexView";
import { DashboardView } from "@loom/ui-kit/components/DashboardView";

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
  const { id } = useParams<{ id: string }>();
  if (id === undefined) return <Navigate to="/dashboards" replace />;

  return (
    <DesktopAppShell>
      <DashboardView
        key={id}
        dashboardId={id}
        onDeleted={() => navigate("/dashboards", { replace: true })}
      />
    </DesktopAppShell>
  );
}
