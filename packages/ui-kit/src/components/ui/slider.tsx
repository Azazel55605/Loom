import * as React from "react";
import * as SliderPrimitive from "@radix-ui/react-slider";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Radix Slider, themed — the replacement for a native `<input type="range">`,
 * per docs/UI_GUIDELINES.md.
 *
 * Radix carries the keyboard behaviour a range input would otherwise provide
 * for free (arrows, Home/End, Page Up/Down) plus the ARIA value attributes, so
 * a themed track loses nothing. Track, range, and thumb all resolve from
 * tokens, so the accent choice reaches the control.
 */
const Slider = React.forwardRef<
  React.ElementRef<typeof SliderPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SliderPrimitive.Root>
>(({ className, ...props }, ref) => (
  <SliderPrimitive.Root
    ref={ref}
    className={cn(
      "relative flex w-full touch-none select-none items-center data-[disabled]:opacity-50",
      className,
    )}
    {...props}
  >
    <SliderPrimitive.Track className="relative h-1.5 w-full grow overflow-hidden rounded-full bg-muted">
      <SliderPrimitive.Range className="absolute h-full bg-primary" />
    </SliderPrimitive.Track>
    <SliderPrimitive.Thumb className="block h-4 w-4 rounded-full border border-primary/50 bg-background shadow transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none" />
  </SliderPrimitive.Root>
));
Slider.displayName = SliderPrimitive.Root.displayName;

export { Slider };
