import * as React from "react";
import { Plug, Settings } from "lucide-react";
import { Link, useLocation, useNavigate } from "react-router-dom";

import { AppShell as SharedAppShell } from "@loom/ui-kit/components/AppShell";
import { DashboardSidebar } from "@loom/ui-kit/components/DashboardSidebar";
import { buttonVariants } from "@loom/ui-kit/components/ui/button";

/** Supplies React Router navigation to the platform-neutral shared shell. */
export function WebAppShell({ children }: { children: React.ReactNode }) {
  const navigate = useNavigate();
  const location = useLocation();
  const dashboardMatch = /^\/dashboards\/([^/]+)$/.exec(location.pathname);

  return (
    <SharedAppShell
      homeControl={
        <Link to="/dashboards" className="text-base font-semibold tracking-tight">
          Loom
        </Link>
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
              className={buttonVariants({ variant: "ghost", size: "sm", className: "w-full justify-start" })}
            >
              <Plug aria-hidden="true" />
              Manage connectors
            </Link>
          }
        />
      }
    >
      {children}
    </SharedAppShell>
  );
}
