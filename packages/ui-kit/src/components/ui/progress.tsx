import * as React from "react";
import * as ProgressPrimitive from "@radix-ui/react-progress";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Radix Progress, themed — a determinate bar for a bounded value.
 *
 * The fill reads from `--accent` via Tailwind's `primary`, so it follows the
 * user's accent colour. Radix owns the ARIA wiring (`role="progressbar"` and
 * the value attributes), which is the whole reason this is not a styled `div`:
 * a screen reader gets the number without the caller doing anything.
 *
 * Motion is a `transform` transition on the indicator, which the global
 * reduced-motion rule collapses to an instant swap. Nothing is conveyed by the
 * movement alone — the fill width carries the value either way.
 */
const Progress = React.forwardRef<
  React.ElementRef<typeof ProgressPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof ProgressPrimitive.Root>
>(({ className, value, ...props }, ref) => (
  <ProgressPrimitive.Root
    ref={ref}
    className={cn(
      "relative h-2 w-full overflow-hidden rounded-full bg-muted",
      className,
    )}
    value={value}
    {...props}
  >
    <ProgressPrimitive.Indicator
      className="h-full w-full flex-1 bg-primary transition-transform"
      style={{ transform: `translateX(-${100 - (value ?? 0)}%)` }}
    />
  </ProgressPrimitive.Root>
));
Progress.displayName = ProgressPrimitive.Root.displayName;

export { Progress };
