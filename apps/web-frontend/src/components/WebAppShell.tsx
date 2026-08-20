import * as React from "react";
import { Settings } from "lucide-react";
import { Link } from "react-router-dom";

import { AppShell as SharedAppShell } from "@loom/ui-kit/components/AppShell";
import { buttonVariants } from "@loom/ui-kit/components/ui/button";

/** Supplies React Router navigation to the platform-neutral shared shell. */
export function WebAppShell({ children }: { children: React.ReactNode }) {
  return (
    <SharedAppShell
      homeControl={
        <Link to="/" className="text-base font-semibold tracking-tight">
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
    >
      {children}
    </SharedAppShell>
  );
}
