import * as React from "react";
import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import { Check } from "lucide-react";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Radix Checkbox, themed — the replacement for a browser-default checkbox, per
 * docs/UI_GUIDELINES.md.
 *
 * Radix renders a hidden native input for form participation and keyboard
 * behaviour while the visible box is ours, so the accent and radius tokens
 * apply without giving up the platform semantics.
 */
const Checkbox = React.forwardRef<
  React.ElementRef<typeof CheckboxPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof CheckboxPrimitive.Root>
>(({ className, ...props }, ref) => (
  <CheckboxPrimitive.Root
    ref={ref}
    className={cn(
      "group peer relative inline-flex h-4 w-4 min-h-11 min-w-11 shrink-0 items-center justify-center rounded-md text-primary-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
      className,
    )}
    {...props}
  >
    <span
      aria-hidden="true"
      className="pointer-events-none absolute size-4 rounded-sm border border-input shadow group-data-[state=checked]:border-primary group-data-[state=checked]:bg-primary"
    />
    <CheckboxPrimitive.Indicator
      className={cn("relative z-[1] flex size-4 items-center justify-center text-current")}
    >
      <Check className="h-3.5 w-3.5" aria-hidden="true" />
    </CheckboxPrimitive.Indicator>
  </CheckboxPrimitive.Root>
));
Checkbox.displayName = CheckboxPrimitive.Root.displayName;

export { Checkbox };
