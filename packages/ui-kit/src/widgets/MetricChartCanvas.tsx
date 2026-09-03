import * as React from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Line,
  LineChart,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  type TooltipContentProps,
  XAxis,
  YAxis,
} from "recharts";

import type { ChartType } from "@loom/ui-kit/lib/api";
import {
  configNumber,
  configString,
  formatNumericReadingText,
  readCategoryBreakdown,
  readNumber,
  readTimeSeries,
  type DisplayWidgetProps,
} from "@loom/ui-kit/widgets/types";
import { ChartPlaceholder } from "@loom/ui-kit/widgets/ChartPlaceholder";

/**
 * The recharts half of `MetricChartWidget`, in its own module so it can be
 * loaded on demand. See that component for why.
 */
export default function MetricChartCanvas({
  label,
  unit,
  value,
  config,
  chartType,
}: Omit<DisplayWidgetProps, "className"> & { chartType: ChartType }) {
  const formatValue = (entry: unknown) =>
    typeof entry === "number" ? formatNumericReadingText(entry, unit) : formatEntry(entry);
  const tooltipContent = React.useCallback(
    (props: TooltipContentProps) => (
      <MetricChartTooltip {...props} fallbackLabel={label} unit={unit} />
    ),
    [label, unit],
  );
  const horizontalBars =
    chartType === "bar" && configString(config, "orientation", "vertical") === "horizontal";

  // Either a chart to draw or a reason there is nothing to draw. Kept as two
  // separate results rather than one element so the container below does not
  // have to inspect what it was handed.
  const rendered: { empty: string } | { chart: React.ReactElement } = (() => {
    if (chartType === "line") {
      const samples = readTimeSeries(value);
      if (samples.length === 0) return { empty: "No readings yet." };

      const data = samples.map((sample) => ({
        time: new Date(sample.timestamp).toLocaleTimeString(),
        value: sample.value,
      }));

      return {
        chart: (
        <LineChart data={data} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid strokeDasharray="3 3" className="stroke-border" vertical={false} />
          <XAxis
            dataKey="time"
            tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
            tickLine={false}
            axisLine={false}
            minTickGap={24}
          />
          <YAxis
            tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }}
            tickLine={false}
            axisLine={false}
            width={36}
            tickFormatter={formatValue}
          />
          <Tooltip content={tooltipContent} cursor={false} />
          <Line
            type="monotone"
            dataKey="value"
            stroke="hsl(var(--accent))"
            strokeWidth={2}
            dot={false}
            isAnimationActive={false}
          />
        </LineChart>
        ),
      };
    }

    // Bar and pie charts may receive a connector-supplied categorical series.
    // Scalar behavior remains intact below for existing descriptors.
    const categories = readCategoryBreakdown(value);
    if (Array.isArray(value)) {
      if (categories.length === 0) return { empty: "No categories yet." };

      if (chartType === "pie") {
        return {
          chart: (
            <PieChart>
              <Pie
                data={categories}
                dataKey="value"
                nameKey="label"
                innerRadius="45%"
                outerRadius="80%"
                isAnimationActive={false}
                stroke="none"
              >
                {categories.map((category, index) => (
                  <Cell
                    key={category.label}
                    fill={CATEGORY_COLORS[index % CATEGORY_COLORS.length]}
                  />
                ))}
              </Pie>
              <Tooltip content={tooltipContent} cursor={false} />
            </PieChart>
          ),
        };
      }

      return {
        chart: (
          <BarChart
            data={categories}
            layout={horizontalBars ? "vertical" : "horizontal"}
            margin={{ top: 4, right: 8, bottom: 0, left: 0 }}
          >
            <CartesianGrid
              strokeDasharray="3 3"
              className="stroke-border"
              horizontal={!horizontalBars}
              vertical={horizontalBars}
            />
            {horizontalBars ? (
              <>
                <XAxis type="number" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} tickFormatter={formatValue} />
                <YAxis type="category" dataKey="label" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} width={96} />
              </>
            ) : (
              <>
                <XAxis type="category" dataKey="label" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} />
                <YAxis type="number" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} width={52} tickFormatter={formatValue} />
              </>
            )}
            <Tooltip content={tooltipContent} cursor={false} />
            <Bar
              dataKey="value"
              isAnimationActive={false}
              radius={horizontalBars ? [0, 4, 4, 0] : [4, 4, 0, 0]}
            >
              {categories.map((category, index) => (
                <Cell
                  key={category.label}
                  fill={CATEGORY_COLORS[index % CATEGORY_COLORS.length]}
                />
              ))}
            </Bar>
          </BarChart>
        ),
      };
    }

    // Pie and Bar over a scalar: the reading, and what is left of `config.max`.
    const reading = readNumber(value);
    if (reading === null) return { empty: "No reading." };

    const max = configNumber(config, "max", 100);
    const remainder = Math.max(0, max - reading);
    const data = [
      { name: label, value: reading },
      { name: "Remaining", value: remainder },
    ];

    if (chartType === "pie") {
      return {
        chart: (
        <PieChart>
          <Pie
            data={data}
            dataKey="value"
            nameKey="name"
            innerRadius="55%"
            outerRadius="80%"
            isAnimationActive={false}
            stroke="none"
          >
            <Cell fill="hsl(var(--accent))" />
            <Cell fill="hsl(var(--muted))" />
          </Pie>
          <Tooltip content={tooltipContent} cursor={false} />
        </PieChart>
        ),
      };
    }

    return {
      chart: (
      <BarChart
        data={[{ name: label, value: reading, remainder }]}
        layout={horizontalBars ? "vertical" : "horizontal"}
        margin={{ top: 4, right: 8, bottom: 0, left: 0 }}
      >
        <CartesianGrid
          strokeDasharray="3 3"
          className="stroke-border"
          horizontal={!horizontalBars}
          vertical={horizontalBars}
        />
        {horizontalBars ? (
          <>
            <XAxis type="number" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} tickFormatter={formatValue} />
            <YAxis type="category" dataKey="name" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} width={96} />
          </>
        ) : (
          <>
            <XAxis type="category" dataKey="name" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} />
            <YAxis type="number" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} width={36} tickFormatter={formatValue} />
          </>
        )}
        <Tooltip content={tooltipContent} cursor={false} />
        <Bar dataKey="value" stackId="reading" fill="hsl(var(--accent))" isAnimationActive={false} radius={[0, 0, 0, 0]} />
        <Bar dataKey="remainder" stackId="reading" fill="hsl(var(--muted))" isAnimationActive={false} radius={horizontalBars ? [0, 4, 4, 0] : [4, 4, 0, 0]} />
      </BarChart>
      ),
    };
  })();

  if ("empty" in rendered) return <ChartPlaceholder label={rendered.empty} />;
  return (
    <ResponsiveContainer width="100%" height="100%">
      {rendered.chart}
    </ResponsiveContainer>
  );
}

/** Recharts' stock tooltip brings its own light palette and `value :` row.
 * Keeping the content app-owned makes charts follow the same elevated-surface
 * treatment as Popover and DropdownMenu on every host WebView. */
function MetricChartTooltip({
  active,
  payload,
  label,
  fallbackLabel,
  unit,
}: TooltipContentProps & { fallbackLabel: string; unit?: string | null }) {
  if (!active || payload.length === 0) return null;

  const primary = payload.find((entry) => entry.dataKey === "value") ?? payload[0];
  const title = tooltipCategory(primary?.payload, label, fallbackLabel);
  const formattedValue =
    typeof primary?.value === "number"
      ? formatNumericReadingText(primary.value, unit)
      : formatEntry(primary?.value);

  return (
    <div className="chart-tooltip surface-elevated min-w-24 rounded-md border border-border px-3 py-2 text-xs text-popover-foreground shadow-md">
      <p className="font-medium leading-none">{title}</p>
      <p className="mt-1.5 tabular-nums text-muted-foreground">{formattedValue}</p>
    </div>
  );
}

function tooltipCategory(payload: unknown, label: unknown, fallback: string): string {
  if (typeof payload === "object" && payload !== null) {
    const record = payload as Record<string, unknown>;
    for (const key of ["label", "name", "time"] as const) {
      const value = record[key];
      if (typeof value === "string" && value.length > 0) return value;
    }
  }
  if (typeof label === "string" || typeof label === "number") return String(label);
  return fallback;
}

const CATEGORY_COLORS = [
  "hsl(var(--accent))",
  "hsl(var(--status-healthy))",
  "hsl(var(--status-degraded))",
  "hsl(var(--status-unknown))",
] as const;

/** Recharts hands a tooltip formatter a loosely-typed value that may be absent
 *  or an array; this narrows it to something printable. */
function formatEntry(entry: unknown): string {
  if (typeof entry === "number") {
    return Number.isInteger(entry) ? String(entry) : entry.toFixed(2);
  }
  if (typeof entry === "string") return entry;
  return "—";
}
