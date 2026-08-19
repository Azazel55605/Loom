import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { LogOut } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { getHealth } from "@/lib/api";
import { useAuth } from "@/lib/auth-context";

/**
 * The signed-in chrome: a sticky header carrying the product name, the backend
 * it is talking to, and the way out.
 *
 * Built from existing shadcn primitives (`Button`, `Badge`) plus layout, rather
 * than as a new primitive — there is no interactive behaviour here that Radix
 * would own. The header uses `.surface-elevated` so the blur and
 * reduced-transparency tokens reach it like every other elevated surface, per
 * docs/UI_GUIDELINES.md.
 */
export function AppShell({ children }: { children: React.ReactNode }) {
  const { user, signOut } = useAuth();

  // The backend version, shown so it is obvious which build the frame is
  // pointed at. Its own query rather than part of the connector fetch: it is
  // near-static, so it does not want the connector list's poll interval.
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => getHealth(signal),
    staleTime: 5 * 60 * 1000,
    retry: false,
  });

  return (
    <div className="min-h-screen bg-background text-foreground">
      <header className="surface-elevated sticky top-0 z-10 border-b border-border">
        <div className="mx-auto flex h-14 w-full max-w-5xl items-center gap-3 px-4">
          <span className="text-base font-semibold tracking-tight">Loom</span>

          {health.isSuccess && (
            <Badge variant="outline" title="web-backend core version">
              core v{health.data.core_version}
            </Badge>
          )}

          <div className="ml-auto flex items-center gap-3">
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

      <main className="mx-auto w-full max-w-5xl px-4 py-8">{children}</main>
    </div>
  );
}
