import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { MobileAppShell } from "@/components/MobileAppShell";
import { MobileKioskSettingsCard } from "@/components/MobileKioskSettingsCard";
import { ServerSwitcherTrigger } from "@loom/ui-kit/components/ServerSwitcher";
import { SettingsLayout } from "@loom/ui-kit/components/SettingsLayout";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { GeneralPanel } from "@loom/ui-kit/pages/settings/GeneralPanel";

export function MobileSettingsRoute() {
  const navigate = useNavigate();
  const location = useLocation();
  const section = location.pathname.split("/")[2] ?? "general";

  return (
    <SettingsLayout
      activeSection={section}
      onSectionChange={(value) => navigate(`/settings/${value}`)}
      renderShell={(content) => <MobileAppShell>{content}</MobileAppShell>}
    >
      {section === "general" ? (
        <div className="flex flex-col gap-4">
          <GeneralPanel />
          <MobileKioskSettingsCard />
          <Card>
            <CardHeader>
              <CardTitle className="text-lg">Server</CardTitle>
              <CardDescription>
                Add, rename, remove, or switch between this device&apos;s Loom servers.
                Each profile keeps its own session.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <ServerSwitcherTrigger manageLabel />
            </CardContent>
          </Card>
        </div>
      ) : (
        <Outlet />
      )}
    </SettingsLayout>
  );
}
