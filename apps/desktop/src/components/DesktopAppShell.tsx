import * as React from "react";
import { Plug, Settings } from "lucide-react";
import { Link, useLocation, useNavigate } from "react-router-dom";

import { AppShell } from "@loom/ui-kit/components/AppShell";
import { DashboardSidebar } from "@loom/ui-kit/components/DashboardSidebar";
import { buttonVariants } from "@loom/ui-kit/components/ui/button";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { useDesktopUpdates } from "@/updater/desktop-update-context";

export function DesktopAppShell({ children }: { children: React.ReactNode }) {
  const navigate = useNavigate();
  const location = useLocation();
  const dashboardMatch = /^\/dashboards\/([^/]+)$/.exec(location.pathname);
  const { update } = useDesktopUpdates();

  return (
    <AppShell
      sidebarNavigationKey={location.pathname}
      homeControl={
        <div className="flex items-center gap-2">
          <Link to="/dashboards" className="text-base font-semibold tracking-tight">
            Loom
          </Link>
          <Badge variant="outline">desktop v{__APP_VERSION__}</Badge>
          {update !== null ? (
            <Link to="/settings/updates" aria-label={`Update v${update.version} available`}>
              <Badge variant="secondary">Update available</Badge>
            </Link>
          ) : null}
        </div>
      }
      settingsControl={
        <Link
          to="/settings"
          title="Settings"
          className={buttonVariants({ variant: "ghost", size: "icon" })}
        >
          <Settings aria-hidden="true" />
          <span className="sr-only">Settings</span>
        </Link>
      }
      sidebar={
        <DashboardSidebar
          activeDashboardId={dashboardMatch?.[1]}
          onNavigate={(dashboardId) => navigate(`/dashboards/${dashboardId}`)}
          footerControl={
            <Link
              to="/connectors"
              className={buttonVariants({
                variant: "ghost",
                size: "sm",
                className: "w-full justify-start",
              })}
            >
              <Plug aria-hidden="true" />
              Manage connectors
            </Link>
          }
        />
      }
    >
      {children}
    </AppShell>
  );
}
