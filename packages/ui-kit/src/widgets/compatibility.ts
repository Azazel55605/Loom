import type {
  ActionWidgetType,
  ChartType,
  ConnectorAction,
  DataPointValueType,
  DisplayWidgetType,
} from "@loom/ui-kit/lib/api";

/**
 * Which widgets may legally draw which data, and which may drive which action.
 *
 * These are the tables the binding editor offers from. They exist so a user
 * cannot build a placement that renders as a blank space — a `statusDot` bound
 * to a time series has no dot to colour, and a `metricChart` bound to a boolean
 * has nothing to plot. The backend does not enforce this pairing (it validates
 * that the ids *exist*, not that they suit each other), so this is a usability
 * guard rather than a security one, and a binding hand-written against the API
 * can still be nonsense.
 */

/** A `DisplayWidgetType` collapsed to a plain string, for `Select` values and
 *  lookup keys. The chart variant loses its `chartType` here; that travels
 *  separately. */
export type DisplayWidgetKey =
  | "statTile"
  | "progressBar"
  | "metricChart"
  | "gauge"
  | "statusDot"
  | "logStream";

/** The key of a display widget type, whichever serialized form it arrived in. */
export function displayWidgetKey(widgetType: DisplayWidgetType): DisplayWidgetKey {
  return typeof widgetType === "string" ? widgetType : "metricChart";
}

/** The chart variant of a display widget type, or `null` for the others. */
export function displayChartType(widgetType: DisplayWidgetType): ChartType | null {
  return typeof widgetType === "string" ? null : widgetType.metricChart.chartType;
}

/** Rebuilds a display widget type from an editor's key plus chart choice. */
export function displayWidgetFromKey(
  key: DisplayWidgetKey,
  chartType: ChartType = "line",
): DisplayWidgetType {
  return key === "metricChart" ? { metricChart: { chartType } } : key;
}

const DISPLAY_LABELS: Record<DisplayWidgetKey, string> = {
  statTile: "Stat tile",
  progressBar: "Progress bar",
  metricChart: "Chart",
  gauge: "Gauge",
  statusDot: "Status dot",
  logStream: "Log stream",
};

const ACTION_LABELS: Record<ActionWidgetType, string> = {
  button: "Button",
  toggle: "Toggle",
  slider: "Slider",
  textField: "Text field",
  selector: "Dropdown",
};

/** Human-facing name for a display widget, for the editor's picker. */
export function describeDisplayWidget(key: DisplayWidgetKey): string {
  return DISPLAY_LABELS[key];
}

/** Human-facing name for an action widget, for the editor's picker. */
export function describeActionWidget(widgetType: ActionWidgetType): string {
  return ACTION_LABELS[widgetType];
}

const BY_VALUE_TYPE: Record<DataPointValueType, DisplayWidgetKey[]> = {
  // A number can be a bare figure, a bounded bar, or a dial. Not a chart: one
  // number is not a series, and `metricChart` over a scalar is the degenerate
  // case it only renders at all as a courtesy.
  number: ["statTile", "progressBar", "gauge"],
  // A string is either short enough to show whole or long enough to scroll.
  string: ["statTile", "logStream"],
  // A boolean is a state, and a dot reads faster than the word — but the tile
  // stays available for when the word is what matters.
  bool: ["statusDot", "statTile"],
  // A series has exactly one home.
  timeSeries: ["metricChart"],
};

/** The display widgets that can render a data point of this value type, most
 *  suitable first. */
export function getCompatibleWidgetTypes(valueType: DataPointValueType): DisplayWidgetKey[] {
  return BY_VALUE_TYPE[valueType] ?? ["statTile"];
}

type SchemaProperty = { type?: unknown };
type MinimalSchema = { properties?: Record<string, SchemaProperty> };

/**
 * The action widgets that can drive this action, most specific first.
 *
 * **A heuristic over a deliberately small slice of JSON Schema**, matching what
 * `SchemaForm` actually supports: a top-level `properties` map of `string`,
 * `number`/`integer`, and `boolean` leaves. Anything outside that — nested
 * objects, arrays, several parameters at once, a schema this cannot read at
 * all — falls back to `button`, which is the one widget that can handle any
 * action by opening the generated parameter form.
 *
 * `button` is therefore always in the list, and always last: a purpose-built
 * control beats a dialog when one fits, and the dialog is the thing that works
 * when none does.
 */
export function getCompatibleActionWidgetTypes(action: ConnectorAction): ActionWidgetType[] {
  const schema = action.paramsSchema;
  if (typeof schema !== "object" || schema === null) return ["button"];

  const properties = (schema as MinimalSchema).properties ?? {};
  const names = Object.keys(properties);
  if (names.length !== 1) return ["button"];

  const type = properties[names[0]]?.type;
  if (type === "boolean") return ["toggle", "button"];
  if (type === "number" || type === "integer") return ["slider", "button"];
  if (type === "string") return ["textField", "selector", "button"];
  return ["button"];
}

/** The single parameter name an action takes, when it takes exactly one.
 *
 * The purpose-built action widgets send one value, and this is what they send
 * it under — so a `Toggle` bound to an action expecting `{"enabled": true}`
 * works without the binding having to spell out `config.paramName`. */
export function soleParameterName(action: ConnectorAction): string | null {
  const schema = action.paramsSchema;
  if (typeof schema !== "object" || schema === null) return null;
  const names = Object.keys((schema as MinimalSchema).properties ?? {});
  return names.length === 1 ? names[0] : null;
}
