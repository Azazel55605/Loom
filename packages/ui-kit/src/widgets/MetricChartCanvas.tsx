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
  XAxis,
  YAxis,
} from "recharts";

import type { ChartType } from "@loom/ui-kit/lib/api";
import {
  configNumber,
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
  const suffix = unit ?? "";
  const tooltipStyle: React.CSSProperties = {
    background: "hsl(var(--popover, var(--card)))",
    border: "1px solid hsl(var(--border))",
    borderRadius: "calc(var(--radius) - 4px)",
    color: "hsl(var(--foreground))",
    fontSize: "0.75rem",
  };

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
          />
          <Tooltip
            contentStyle={tooltipStyle}
            formatter={(entry) => [`${formatEntry(entry)}${suffix}`, label]}
          />
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
          <Tooltip
            contentStyle={tooltipStyle}
            formatter={(entry) => `${formatEntry(entry)}${suffix}`}
          />
        </PieChart>
        ),
      };
    }

    return {
      chart: (
      <BarChart data={[{ name: label, value: reading, remainder }]} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
        <CartesianGrid strokeDasharray="3 3" className="stroke-border" vertical={false} />
        <XAxis dataKey="name" tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} />
        <YAxis tick={{ fontSize: 10, fill: "hsl(var(--muted-foreground))" }} tickLine={false} axisLine={false} width={36} />
        <Tooltip contentStyle={tooltipStyle} formatter={(entry) => `${formatEntry(entry)}${suffix}`} />
        <Bar dataKey="value" stackId="reading" fill="hsl(var(--accent))" isAnimationActive={false} radius={[0, 0, 0, 0]} />
        <Bar dataKey="remainder" stackId="reading" fill="hsl(var(--muted))" isAnimationActive={false} radius={[4, 4, 0, 0]} />
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

/** Recharts hands a tooltip formatter a loosely-typed value that may be absent
 *  or an array; this narrows it to something printable. */
function formatEntry(entry: unknown): string {
  if (typeof entry === "number") {
    return Number.isInteger(entry) ? String(entry) : entry.toFixed(2);
  }
  if (typeof entry === "string") return entry;
  return "—";
}
