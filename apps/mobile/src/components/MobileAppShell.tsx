import * as React from "react";
import { LogOut, Plug, Settings } from "lucide-react";
import { Link, useLocation, useNavigate } from "react-router-dom";

import { AppShell } from "@loom/ui-kit/components/AppShell";
import { DashboardSidebar } from "@loom/ui-kit/components/DashboardSidebar";
import { Button, buttonVariants } from "@loom/ui-kit/components/ui/button";
import { useAuth } from "@loom/ui-kit/lib/auth-context";

export function MobileAppShell({ children }: { children: React.ReactNode }) {
  const navigate = useNavigate();
  const location = useLocation();
  const { signOut } = useAuth();
  const dashboardMatch = /^\/dashboards\/([^/]+)$/.exec(location.pathname);

  return (
    <AppShell
      sidebarNavigationKey={location.pathname}
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
            <div className="flex flex-col gap-1">
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
              <Link
                to="/settings"
                className={buttonVariants({
                  variant: "ghost",
                  size: "sm",
                  className: "w-full justify-start",
                })}
              >
                <Settings aria-hidden="true" />
                Settings
              </Link>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="w-full justify-start"
                onClick={() => void signOut()}
              >
                <LogOut aria-hidden="true" />
                Sign out
              </Button>
            </div>
          }
        />
      }
    >
      {children}
    </AppShell>
  );
}
