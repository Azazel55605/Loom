import { cva, type VariantProps } from "class-variance-authority";

import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import {
  formatNumericReading,
  formatReading,
  type DisplayWidgetProps,
} from "@loom/ui-kit/widgets/types";

const statTileValue = cva("font-semibold leading-none tracking-tight tabular-nums", {
  variants: {
    size: {
      sm: "[font-size:calc(var(--stat-tile-font-size)*0.667)]",
      md: "[font-size:var(--stat-tile-font-size)]",
      lg: "[font-size:calc(var(--stat-tile-font-size)*1.2)]",
    },
  },
  defaultVariants: { size: "md" },
});

export function StatTileSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><Skeleton className="h-8 w-24" /><Skeleton className="h-3 w-16" /></div>;
}

/**
 * One reading, shown large, with its label underneath.
 *
 * The plainest of the display primitives and the fallback for anything scalar:
 * a number, a string, or a boolean all render here legibly, which is why the
 * compatibility table offers it for three of the four value types.
 *
 * `tabular-nums` is load-bearing rather than decorative — a value that
 * re-renders on every status frame jitters horizontally without it, and a
 * number that visibly twitches reads as broken.
 *
 * Takes no `config`; the prop is part of the shared display contract and is
 * simply ignored here.
 */
export function StatTileWidget({
  label,
  unit,
  value,
  className,
  size,
}: DisplayWidgetProps & VariantProps<typeof statTileValue>) {
  const numeric = typeof value === "number" ? formatNumericReading(value, unit) : null;
  const text = numeric?.text ?? formatReading(value);
  const displayedUnit = numeric?.unit ?? unit;
  const resolvedSize = size ?? (value === undefined || text.length > 12 ? "sm" : "md");

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-1", className)}>
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-1 gap-y-0">
        <span
          className={cn(
            statTileValue({ size: resolvedSize }),
            "min-w-0 max-w-full whitespace-pre-line [overflow-wrap:anywhere]",
            text.includes("\n") && "leading-snug",
          )}
        >
          {text}
        </span>
        {displayedUnit && value !== undefined ? (
          <span className="shrink-0 text-sm text-muted-foreground">{displayedUnit}</span>
        ) : null}
      </div>
      <span className="whitespace-normal text-xs text-muted-foreground [overflow-wrap:anywhere]">
        {label}
      </span>
    </div>
  );
}
