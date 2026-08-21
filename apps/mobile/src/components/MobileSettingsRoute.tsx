import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { MobileAppShell } from "@/components/MobileAppShell";
import { ConnectToServer } from "@loom/ui-kit/components/ConnectToServer";
import { SettingsLayout } from "@loom/ui-kit/components/SettingsLayout";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { GeneralPanel } from "@loom/ui-kit/pages/settings/GeneralPanel";

export function MobileSettingsRoute({
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
      renderShell={(content) => <MobileAppShell>{content}</MobileAppShell>}
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
                onConnected={({ baseUrl: nextBaseUrl }) =>
                  onServerChanged(nextBaseUrl)
                }
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
