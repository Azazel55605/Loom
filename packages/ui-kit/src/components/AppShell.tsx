import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { LogOut, PanelLeftClose, PanelLeftOpen } from "lucide-react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { cn } from "@loom/ui-kit/lib/utils";

const SIDEBAR_COLLAPSED_KEY = "loom-sidebar-collapsed";

function storedSidebarState(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true";
  } catch {
    return false;
  }
}

/** Shared signed-in chrome with platform navigation supplied as controls. */
export function AppShell({
  homeControl,
  settingsControl,
  sidebar,
  children,
}: {
  homeControl: React.ReactNode;
  settingsControl: React.ReactNode;
  /** Optional platform-neutral navigation rendered beside signed-in content. */
  sidebar?: React.ReactNode;
  children: React.ReactNode;
}) {
  const api = useApiClient();
  const { user, signOut } = useAuth();
  const [sidebarCollapsed, setSidebarCollapsed] = React.useState(storedSidebarState);
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => api.getHealth(signal),
    staleTime: 5 * 60 * 1000,
    retry: false,
  });

  const toggleSidebar = React.useCallback(() => {
    setSidebarCollapsed((current) => {
      const next = !current;
      try {
        localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next));
      } catch {
        // Persistence is best-effort. The control must still work when storage
        // is unavailable, such as in a strict private-browsing context.
      }
      return next;
    });
  }, []);

  React.useEffect(() => {
    if (sidebar === undefined) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "b") {
        event.preventDefault();
        toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [sidebar, toggleSidebar]);

  return (
    <div className="app-canvas min-h-screen bg-background text-foreground">
      <header className="surface-elevated sticky top-0 z-10 border-b border-border">
        <div className="flex h-14 w-full items-center gap-3 px-4">
          {sidebar === undefined ? null : (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              aria-expanded={!sidebarCollapsed}
              aria-controls="app-sidebar"
              onClick={toggleSidebar}
            >
              {sidebarCollapsed ? (
                <PanelLeftOpen data-icon="inline-start" aria-hidden="true" />
              ) : (
                <PanelLeftClose data-icon="inline-start" aria-hidden="true" />
              )}
            </Button>
          )}
          {homeControl}

          {health.isSuccess && (
            <Badge variant="outline" title="web-backend core version">
              core v{health.data.core_version}
            </Badge>
          )}

          <div className="ml-auto flex items-center gap-3">
            {settingsControl}
            {user !== null && (
              <span className="hidden text-sm text-muted-foreground sm:inline">
                {user.username}
              </span>
            )}
            <Button variant="ghost" size="sm" onClick={() => void signOut()}>
              <LogOut aria-hidden="true" />
              Sign out
            </Button>
          </div>
        </div>
      </header>

      {sidebar === undefined ? (
        <main className="mx-auto w-full max-w-5xl px-4 py-8">{children}</main>
      ) : (
        <div className="flex w-full flex-col md:min-h-[calc(100vh-3.5rem)] md:flex-row">
          <aside
            id="app-sidebar"
            className={cn(
              "surface-panel shrink-0 overflow-hidden border-b transition-[width] md:sticky md:top-14 md:h-[calc(100vh-3.5rem)] md:overflow-y-auto md:border-b-0 md:border-r",
              sidebarCollapsed
                ? "hidden md:block md:w-0 md:border-r-0"
                : "w-full md:w-64",
            )}
          >
            {sidebarCollapsed ? null : sidebar}
          </aside>
          <main className="min-w-0 flex-1">
            <div className="mx-auto w-full max-w-7xl px-4 py-8 md:px-6">{children}</div>
          </main>
        </div>
      )}
    </div>
  );
}
