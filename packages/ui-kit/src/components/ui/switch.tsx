import * as React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Radix Switch, themed — the replacement for a native checkbox used as a
 * toggle, per docs/UI_GUIDELINES.md.
 *
 * The checked state reads from `--accent` via Tailwind's `primary`, so it
 * follows the user's accent colour. Motion is a `translate-x` transition, which
 * the global `prefers-reduced-motion` rule in `index.css` reduces to an instant
 * swap — the state stays visible either way, because the thumb's position and
 * the track's colour both convey it.
 */
const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitive.Root
    ref={ref}
    className={cn(
      "group peer relative inline-flex h-5 w-9 min-h-11 min-w-11 shrink-0 cursor-pointer items-center justify-center rounded-md focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50",
      className,
    )}
    {...props}
  >
    <span
      aria-hidden="true"
      className="pointer-events-none absolute h-5 w-9 rounded-full border-2 border-transparent bg-muted shadow-sm transition-colors group-data-[state=checked]:bg-primary"
    />
    <SwitchPrimitive.Thumb
      className={cn(
        "pointer-events-none relative z-[1] block h-4 w-4 -translate-x-2 rounded-full bg-background shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-2",
      )}
    />
  </SwitchPrimitive.Root>
));
Switch.displayName = SwitchPrimitive.Root.displayName;

export { Switch };
