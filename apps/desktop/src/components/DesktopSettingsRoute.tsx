import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { ConnectToServer } from "@/components/ConnectToServer";
import { DesktopAppShell } from "@/components/DesktopAppShell";
import { SettingsLayout } from "@loom/ui-kit/components/SettingsLayout";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { GeneralPanel } from "@loom/ui-kit/pages/settings/GeneralPanel";

export function DesktopSettingsRoute({
  baseUrl,
  onServerChanged,
}: {
  baseUrl: string;
  onServerChanged: (baseUrl: string) => Promise<void>;
}) {
  const navigate = useNavigate();
  const location = useLocation();
  const section = location.pathname.split("/")[2] ?? "general";

  return (
    <SettingsLayout
      activeSection={section}
      onSectionChange={(value) => navigate(`/settings/${value}`)}
      renderShell={(content) => <DesktopAppShell>{content}</DesktopAppShell>}
    >
      {section === "general" ? (
        <div className="flex flex-col gap-4">
          <GeneralPanel />
          <Card>
            <CardHeader>
              <CardTitle className="text-lg">Server</CardTitle>
              <CardDescription>
                Changing server signs this device out before connecting to the new
                instance.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <ConnectToServer
                embedded
                initialUrl={baseUrl}
                onConnected={onServerChanged}
              />
            </CardContent>
          </Card>
        </div>
      ) : (
        <Outlet />
      )}
    </SettingsLayout>
  );
}
