import * as React from "react";

import { cn } from "@loom/ui-kit/lib/utils";
import type { ChartType } from "@loom/ui-kit/lib/api";
import { ChartPlaceholder } from "@loom/ui-kit/widgets/ChartPlaceholder";
import type { DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

/**
 * The chart library, loaded on demand.
 *
 * Recharts is around half a megabyte and reaches the app through this one
 * widget. Bundled eagerly it would more than double the main chunk for every
 * user, including the ones whose dashboards contain no chart at all — so the
 * whole recharts half lives in `MetricChartCanvas` and arrives only when a
 * chart is actually rendered. The suspense fallback is the same placeholder an
 * empty chart shows, so the load reads as "no data yet" rather than as a
 * flicker of missing layout.
 */
const MetricChartCanvas = React.lazy(() => import("@loom/ui-kit/widgets/MetricChartCanvas"));

/**
 * A data point plotted.
 *
 * `chartType` is a **prop, not a config key**, because in the corrected binding
 * model it is part of the widget type itself — `{"metricChart": {"chartType":
 * "line"}}` — so the dispatcher already knows it, and reading it back out of
 * the free-form `config` would give a hand-edited binding two disagreeing
 * answers.
 *
 * ## What each variant is actually for
 *
 * **Line** is the one with a real job today: it plots a `timeSeries` reading,
 * the `{timestamp, value}[]` shape from `ConnectorStatus.details`, which is why
 * the compatibility table offers `metricChart` for that value type and nothing
 * else.
 *
 * **Pie and Bar want multi-series data that no connector produces yet.** A
 * connector reporting "20 GiB used of 50 GiB" as one data point has one number,
 * and there is nothing to compare it against without inventing a second. So
 * they render the honest minimum: the reading against `config.max`, as one
 * filled segment or one stacked bar with its remainder. They become genuinely
 * useful when a data point can carry categories — which is a connector-contract
 * change, not a widget change, and deliberately not pre-built for here.
 *
 * Colours come from the token palette through CSS variables, never from
 * Recharts' defaults, so a chart follows the accent choice and inverts with the
 * dark palette like everything else. Animation is off throughout: a series that
 * re-animates on every status frame is unreadable, and the movement conveys
 * nothing the line does not already show.
 */
export function MetricChartWidget({
  label,
  unit,
  value,
  config,
  chartType,
  className,
}: DisplayWidgetProps & { chartType: ChartType }) {
  return (
    <div className={cn("flex min-h-0 min-w-0 flex-col gap-1", className)}>
      <span className="truncate text-xs text-muted-foreground" title={label}>
        {label}
      </span>
      <div className="min-h-[6rem] flex-1">
        <React.Suspense fallback={<ChartPlaceholder label="Loading chart…" />}>
          <MetricChartCanvas
            label={label}
            unit={unit}
            value={value}
            config={config}
            chartType={chartType}
          />
        </React.Suspense>
      </div>
    </div>
  );
}
