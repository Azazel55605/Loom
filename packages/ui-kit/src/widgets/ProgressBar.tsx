import { Progress } from "@loom/ui-kit/components/ui/progress";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { configNumber, readNumber, type DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

export function ProgressBarSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><div className="flex justify-between"><Skeleton className="h-3 w-20" /><Skeleton className="h-4 w-10" /></div><Skeleton className="h-4 w-full rounded-full" /></div>;
}

/**
 * A bounded numeric reading as a filled bar.
 *
 * The bar is a percentage of `config.min`–`config.max` (0–100 by default), but
 * the *number* shown alongside it is the raw reading with its own unit. A bar
 * that silently converted 6 GiB of 16 GiB into "37%" and showed only that would
 * lose the value the connector actually reported.
 *
 * A reading outside the configured bounds clamps the bar but not the label, so
 * a misconfigured `max` looks wrong rather than looking fine.
 */
export function ProgressBarWidget({ label, unit, value, config, className }: DisplayWidgetProps) {
  const reading = readNumber(value);
  const min = configNumber(config, "min", 0);
  const max = configNumber(config, "max", 100);
  const span = max - min;

  const percent =
    reading === null || span <= 0
      ? 0
      : Math.min(100, Math.max(0, ((reading - min) / span) * 100));

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-2", className)}>
      <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-2 gap-y-1">
        <span className="min-w-0 whitespace-normal text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {label}
        </span>
        <span className="max-w-full text-sm font-medium tabular-nums [overflow-wrap:anywhere]">
          {reading === null ? "—" : `${Number.isInteger(reading) ? reading : reading.toFixed(1)}${unit ?? ""}`}
        </span>
      </div>
      <Progress
        value={percent}
        aria-label={label}
        // The raw reading, not the derived percentage: a screen reader should
        // hear what the connector said.
        aria-valuetext={reading === null ? "No reading" : `${reading}${unit ?? ""}`}
      />
    </div>
  );
}
