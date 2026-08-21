import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { LogOut } from "lucide-react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";

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
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => api.getHealth(signal),
    staleTime: 5 * 60 * 1000,
    retry: false,
  });

  return (
    <div className="app-canvas min-h-screen bg-background text-foreground">
      <header className="surface-elevated sticky top-0 z-10 border-b border-border">
        <div className="mx-auto flex h-14 w-full max-w-7xl items-center gap-3 px-4">
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
        <div className="mx-auto flex w-full max-w-7xl flex-col md:min-h-[calc(100vh-3.5rem)] md:flex-row">
          <aside className="surface-panel shrink-0 border-b md:sticky md:top-14 md:h-[calc(100vh-3.5rem)] md:w-64 md:overflow-y-auto md:border-b-0 md:border-r">
            {sidebar}
          </aside>
          <main className="min-w-0 flex-1 px-4 py-8 md:px-6">{children}</main>
        </div>
      )}
    </div>
  );
}
