import * as React from "react";
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { Circle } from "lucide-react";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Radix RadioGroup, themed — the accessible construct for "exactly one of
 * these", and the themed replacement for a browser-default radio.
 *
 * Radix supplies the part that is tedious and easy to get wrong: roving focus,
 * so the group is one tab stop and the arrow keys move between options, plus
 * the `aria-checked` wiring. A set of buttons with `aria-pressed` looks the same
 * and behaves differently — each becomes its own tab stop, and pressed is not
 * the same statement as selected.
 */
const RadioGroup = React.forwardRef<
  React.ElementRef<typeof RadioGroupPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Root>
>(({ className, ...props }, ref) => (
  <RadioGroupPrimitive.Root ref={ref} className={cn("grid gap-2", className)} {...props} />
));
RadioGroup.displayName = RadioGroupPrimitive.Root.displayName;

const RadioGroupItem = React.forwardRef<
  React.ElementRef<typeof RadioGroupPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof RadioGroupPrimitive.Item>
>(({ className, ...props }, ref) => (
  <RadioGroupPrimitive.Item
    ref={ref}
    className={cn(
      "relative inline-flex size-4 min-h-[var(--touch-target-size)] min-w-[var(--touch-target-size)] shrink-0 items-center justify-center rounded-md text-primary focus:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
      className,
    )}
    {...props}
  >
    <span aria-hidden="true" className="pointer-events-none absolute size-4 rounded-full border border-input shadow" />
    <RadioGroupPrimitive.Indicator className="relative flex size-4 items-center justify-center">
      <Circle className="size-2 fill-current text-current" aria-hidden="true" />
    </RadioGroupPrimitive.Indicator>
  </RadioGroupPrimitive.Item>
));
RadioGroupItem.displayName = RadioGroupPrimitive.Item.displayName;

export { RadioGroup, RadioGroupItem };
