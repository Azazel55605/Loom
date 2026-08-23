/** A labelled empty state where a chart would be — no reading yet, or the
 *  chart module still loading. Its own module so both `MetricChart` and the
 *  lazily-loaded `MetricChartCanvas` can show it without the wrapper having to
 *  pull the chart library in to render "nothing to draw". */
export function ChartPlaceholder({ label }: { label: string }) {
  return (
    <div className="flex h-full min-h-[6rem] items-center justify-center rounded-md border border-dashed text-xs text-muted-foreground">
      {label}
    </div>
  );
}
