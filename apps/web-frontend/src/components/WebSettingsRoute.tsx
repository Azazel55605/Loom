import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { SettingsLayout as SharedSettingsLayout } from "@loom/ui-kit/components/SettingsLayout";
import { WebAppShell } from "@/components/WebAppShell";

export function WebSettingsRoute() {
  const navigate = useNavigate();
  const location = useLocation();
  const section = location.pathname.split("/")[2] ?? "general";

  return (
    <SharedSettingsLayout
      activeSection={section}
      onSectionChange={(value) => navigate(`/settings/${value}`)}
      renderShell={(content) => <WebAppShell>{content}</WebAppShell>}
    >
      <Outlet />
    </SharedSettingsLayout>
  );
}
