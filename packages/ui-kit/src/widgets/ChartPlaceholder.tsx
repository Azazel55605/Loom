/** A labelled empty state where a chart would be — no reading yet, or the
 *  chart module still loading. Its own module so both `MetricChart` and the
 *  lazily-loaded `MetricChartCanvas` can show it without the wrapper having to
 *  pull the chart library in to render "nothing to draw". */
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";

export function ChartPlaceholder({ label, loading = false }: { label: string; loading?: boolean }) {
  return (
    <div className="flex h-full min-h-[6rem] items-center justify-center rounded-md border border-dashed text-xs text-muted-foreground" aria-label={label}>
      {loading ? <Skeleton className="h-2/3 w-4/5" /> : label}
    </div>
  );
}
