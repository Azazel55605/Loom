import * as React from "react";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Keyed route/tab content entrance. The root animation setting decides whether
 * this is a directional transition, a short opacity-only transition, or an
 * instant swap; callers only provide the identity that changed.
 */
export function MotionContent({
  motionKey,
  variant = "page",
  className,
  children,
}: {
  motionKey: React.Key;
  variant?: "page" | "tab";
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      key={motionKey}
      className={cn(
        variant === "page" ? "motion-page-content" : "motion-tab-content",
        className,
      )}
    >
      {children}
    </div>
  );
}
