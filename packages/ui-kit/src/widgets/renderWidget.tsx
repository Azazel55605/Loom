import { AlertTriangle } from "lucide-react";

import type {
  ConnectorAction,
  DataPointDescriptor,
  WidgetBinding,
} from "@loom/ui-kit/lib/api";
import { GaugeWidget } from "@loom/ui-kit/widgets/Gauge";
import { LogStreamWidget } from "@loom/ui-kit/widgets/LogStream";
import { MetricChartWidget } from "@loom/ui-kit/widgets/MetricChart";
import { ProgressBarWidget } from "@loom/ui-kit/widgets/ProgressBar";
import { StatTileWidget } from "@loom/ui-kit/widgets/StatTile";
import { StatusDotWidget } from "@loom/ui-kit/widgets/StatusDot";
import { ActionButtonWidget } from "@loom/ui-kit/widgets/ActionButton";
import { ActionSelectorWidget } from "@loom/ui-kit/widgets/ActionSelector";
import { ActionSliderWidget } from "@loom/ui-kit/widgets/ActionSlider";
import { ActionTextFieldWidget } from "@loom/ui-kit/widgets/ActionTextField";
import { ActionToggleWidget } from "@loom/ui-kit/widgets/ActionToggle";
import { soleParameterName } from "@loom/ui-kit/widgets/compatibility";
import type { WidgetExecute } from "@loom/ui-kit/widgets/types";

/** Everything one binding needs to become a widget. */
export type RenderWidgetOptions = {
  binding: WidgetBinding;
  /** `status.details` for this placement's connector, or `{}` when there is no
   *  reading. Keyed by data point id, per the `ConnectorStatus.details`
   *  contract. */
  statusDetails: Record<string, unknown>;
  /** The connector's declared data points, for labels and units. */
  dataPoints: DataPointDescriptor[];
  /** The connector's declared actions, for labels and parameter schemas. */
  actions: ConnectorAction[];
  onExecute: WidgetExecute;
  /** Disables every control. Set for a viewer without `connectors.control`.
   *  Visibility only — the backend re-checks each request. */
  disabled?: boolean;
  className?: string;
};

/**
 * Turns one `WidgetBinding` into the component that draws it.
 *
 * The single place the binding model meets the primitives, and the reason
 * neither `DashboardView` nor any widget has to know both. It narrows on the
 * binding's tag first — `display` resolves against `dataPoints` and reads
 * `statusDetails`, `action` resolves against `actions` and gets `onExecute` —
 * which is the split the corrected `WidgetBinding` enum exists to make
 * checkable (see `docs/adr/0014-widget-binding-model.md`).
 *
 * A binding naming something the connector no longer declares renders a visible
 * note rather than nothing. A connector's data points can change with its
 * configuration, and a placement saved against an older shape should say so on
 * screen — a silently missing widget is indistinguishable from a layout bug.
 *
 * Takes one options object rather than five positional arguments; every call
 * site passes the same connector-wide values to every binding in a loop, and
 * naming them at the call site is what keeps that loop readable.
 */
export function renderWidget({
  binding,
  statusDetails,
  dataPoints,
  actions,
  onExecute,
  disabled,
  className,
}: RenderWidgetOptions) {
  if ("display" in binding) {
    const { dataPointId, widgetType, config } = binding.display;
    const descriptor = dataPoints.find((point) => point.id === dataPointId);
    if (descriptor === undefined) {
      return <MissingBinding what="data point" id={dataPointId} className={className} />;
    }

    const shared = {
      label: descriptor.label,
      unit: descriptor.unit,
      value: statusDetails[dataPointId],
      config,
      className,
    };

    if (typeof widgetType !== "string") {
      return <MetricChartWidget {...shared} chartType={widgetType.metricChart.chartType} />;
    }

    switch (widgetType) {
      case "statTile":
        return <StatTileWidget {...shared} />;
      case "progressBar":
        return <ProgressBarWidget {...shared} />;
      case "gauge":
        return <GaugeWidget {...shared} />;
      case "statusDot":
        return <StatusDotWidget {...shared} />;
      case "logStream":
        return <LogStreamWidget {...shared} />;
      default:
        // Unreachable while the union and this switch agree. Kept because they
        // are updated in different repositories' worth of code — Core adds the
        // variant, this file draws it — and the gap between those two commits
        // should look like a labelled hole, not a crash.
        return <MissingBinding what="widget type" id={String(widgetType)} className={className} />;
    }
  }

  const { actionId, widgetType, config } = binding.action;
  const action = actions.find((candidate) => candidate.id === actionId);
  if (action === undefined) {
    return <MissingBinding what="action" id={actionId} className={className} />;
  }

  const shared = {
    label: action.label,
    actionId,
    description: action.description,
    paramsSchema: action.paramsSchema,
    // The action's own parameter name, when it takes exactly one, so a binding
    // does not have to restate it. An explicit `config.paramName` still wins.
    config: { paramName: soleParameterName(action) ?? "value", ...asObject(config) },
    onExecute,
    disabled,
    className,
  };

  switch (widgetType) {
    case "button":
      return <ActionButtonWidget {...shared} />;
    case "toggle":
      return <ActionToggleWidget {...shared} />;
    case "slider":
      return <ActionSliderWidget {...shared} />;
    case "textField":
      return <ActionTextFieldWidget {...shared} />;
    case "selector":
      return <ActionSelectorWidget {...shared} />;
    default:
      return <MissingBinding what="widget type" id={String(widgetType)} className={className} />;
  }
}

function asObject(config: unknown): Record<string, unknown> {
  return typeof config === "object" && config !== null && !Array.isArray(config)
    ? (config as Record<string, unknown>)
    : {};
}

function MissingBinding({
  what,
  id,
  className,
}: {
  what: string;
  id: string;
  className?: string;
}) {
  return (
    <div
      className={`flex min-w-0 items-center gap-2 rounded-md border border-dashed p-2 text-xs text-muted-foreground ${className ?? ""}`}
    >
      <AlertTriangle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="truncate">
        Unknown {what} <code className="font-mono">{id}</code>
      </span>
    </div>
  );
}
