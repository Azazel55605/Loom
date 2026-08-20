import * as React from "react";

import { RadioGroup } from "@loom/ui-kit/components/ui/radio-group";
import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * A small set of mutually exclusive choices, shown side by side.
 *
 * Built on Radix `RadioGroup` rather than a row of buttons, because that is
 * what this is: exactly one option is selected, arrow keys should move between
 * them, and the whole group should be a single tab stop. It renders the items
 * as labelled segments instead of dots — the same primitive, a different skin,
 * which is step (b) of the sourcing rule in docs/UI_GUIDELINES.md.
 *
 * Preferred over `Select` for three-or-fewer short options: every choice is
 * visible without opening anything, and the comparison between them is the
 * point.
 */
export type Segment<T extends string> = {
  value: T;
  label: string;
  /** Optional leading icon. Decorative — the label carries the meaning. */
  icon?: React.ReactNode;
};

export function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T;
  onChange: (value: T) => void;
  options: Segment<T>[];
  /** Accessible name for the group, since the segments only label themselves. */
  label: string;
  className?: string;
}) {
  return (
    <RadioGroup
      aria-label={label}
      value={value}
      onValueChange={(next) => onChange(next as T)}
      className={cn(
        "surface-panel inline-flex w-fit grid-flow-col gap-1 rounded-lg border p-1",
        className,
      )}
    >
      {options.map((option) => (
        <RadioGroupPrimitive.Item
          key={option.value}
          value={option.value}
          className={cn(
            "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
            "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
            // Selected reads as a raised surface, matching how `TabsTrigger`
            // shows its active state, so the two controls look related.
            "text-muted-foreground data-[state=checked]:bg-card data-[state=checked]:text-foreground data-[state=checked]:shadow",
            "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
          )}
        >
          {option.icon}
          {option.label}
        </RadioGroupPrimitive.Item>
      ))}
    </RadioGroup>
  );
}
