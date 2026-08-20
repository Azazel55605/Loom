import { Navigate, Outlet, useLocation, useNavigate } from "react-router-dom";

import {
  PermissionsLayout,
  usePreferredPermissionsSection,
} from "@loom/ui-kit/components/PermissionsLayout";

export function DesktopPermissionsRoute() {
  const navigate = useNavigate();
  const location = useLocation();
  const section = location.pathname.split("/")[3] ?? "users";

  return (
    <PermissionsLayout
      activeSection={section}
      onSectionChange={(value) => navigate(`/settings/permissions/${value}`)}
    >
      <Outlet />
    </PermissionsLayout>
  );
}

export function DesktopPermissionsIndexRedirect() {
  const section = usePreferredPermissionsSection();
  return <Navigate to={section} replace />;
}
