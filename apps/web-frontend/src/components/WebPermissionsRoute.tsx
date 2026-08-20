import { Navigate, Outlet, useLocation, useNavigate } from "react-router-dom";

import {
  PermissionsLayout as SharedPermissionsLayout,
  usePreferredPermissionsSection,
} from "@loom/ui-kit/components/PermissionsLayout";

export function WebPermissionsRoute() {
  const navigate = useNavigate();
  const location = useLocation();
  const section = location.pathname.split("/")[3] ?? "users";

  return (
    <SharedPermissionsLayout
      activeSection={section}
      onSectionChange={(value) => navigate(`/settings/permissions/${value}`)}
    >
      <Outlet />
    </SharedPermissionsLayout>
  );
}

export function WebPermissionsIndexRedirect() {
  const section = usePreferredPermissionsSection();
  return <Navigate to={section} replace />;
}
