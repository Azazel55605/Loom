/**
 * Shared prop contracts and config readers for the widget primitives.
 *
 * Every widget takes its *descriptor* (what this thing is called) separately
 * from its *value* (what it currently reads). That mirrors the backend split —
 * `dataPoints()` describes, `status.details` supplies — and it is what lets a
 * widget re-render on a WebSocket status frame without re-reading any schema.
 *
 * `config` is `unknown` on purpose. It is the free-form per-binding object from
 * `WidgetBinding`, authored in the binding editor and never validated by the
 * server, so a widget that trusted its shape would break on hand-edited data.
 * The readers below are the only sanctioned way in: each one takes a fallback
 * and returns it whenever the stored value is missing or the wrong type.
 */

/** Runs one connector action. Resolves on success, rejects on failure — the
 *  optimistic widgets rely on the rejection to roll themselves back. */
export type WidgetExecute = (
  actionId: string,
  params: Record<string, unknown>,
) => Promise<unknown>;

/** What every display widget receives. */
export type DisplayWidgetProps = {
  /** The data point's human-facing label, for the widget's own caption. */
  label: string;
  /** Display suffix (`"%"`, `"MiB"`), or `null` for a dimensionless value. */
  unit?: string | null;
  /** The current reading, straight from `status.details[dataPointId]`.
   *  `undefined` when the connector has not reported this data point — which is
   *  a state every widget must render, not an error. */
  value: unknown;
  /** The binding's free-form `config`. Read it through the helpers below. */
  config: unknown;
  className?: string;
};

/** What every action widget receives. */
export type ActionWidgetProps = {
  /** The action's human-facing label. */
  label: string;
  /** The `ConnectorAction.id` to invoke. */
  actionId: string;
  /** The action's own JSON Schema, for the widgets that need to know whether
   *  there is anything to collect before running. */
  paramsSchema?: unknown;
  /** Longer explanation, used as a dialog description or a title attribute. */
  description?: string | null;
  /** The binding's free-form `config`. */
  config: unknown;
  onExecute: WidgetExecute;
  /** Set while the caller has no `connectors.control` grant, or while another
   *  action on the same card is already running. Visibility only — the backend
   *  re-checks every request. */
  disabled?: boolean;
  className?: string;
};

function record(config: unknown): Record<string, unknown> {
  return typeof config === "object" && config !== null && !Array.isArray(config)
    ? (config as Record<string, unknown>)
    : {};
}

/** A finite number from `config[key]`, or `fallback`. */
export function configNumber(config: unknown, key: string, fallback: number): number {
  const value = record(config)[key];
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/** A non-empty string from `config[key]`, or `fallback`. */
export function configString(config: unknown, key: string, fallback: string): string {
  const value = record(config)[key];
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

/** A list of strings from `config[key]`, dropping any non-string entries. */
export function configStringArray(config: unknown, key: string): string[] {
  const value = record(config)[key];
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

/** A `string -> string` map from `config[key]`, dropping malformed entries. */
export function configStringMap(config: unknown, key: string): Record<string, string> {
  const entries = Object.entries(record(record(config)[key]));
  const result: Record<string, string> = {};
  for (const [name, value] of entries) {
    if (typeof value === "string") result[name] = value;
  }
  return result;
}

/** One point of a `timeSeries` reading, per the `ConnectorStatus.details`
 *  contract in docs/API_CONTRACT.md. */
export type TimeSeriesSample = {
  timestamp: string;
  value: number;
};

/**
 * Reads a `timeSeries` value, keeping only well-formed samples.
 *
 * A connector that breaks the contract yields a shorter series or an empty one
 * rather than a chart of `NaN`, which renders as an axis with no line and is at
 * least honest about having nothing to draw.
 */
export function readTimeSeries(value: unknown): TimeSeriesSample[] {
  if (!Array.isArray(value)) return [];
  const samples: TimeSeriesSample[] = [];
  for (const entry of value) {
    if (typeof entry !== "object" || entry === null) continue;
    const sample = entry as Partial<TimeSeriesSample>;
    if (typeof sample.timestamp !== "string" || typeof sample.value !== "number") continue;
    if (!Number.isFinite(sample.value)) continue;
    samples.push({ timestamp: sample.timestamp, value: sample.value });
  }
  return samples;
}

/** A finite number from a reading, or `null` when there is nothing usable. */
export function readNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/**
 * Formats a reading for a text display.
 *
 * Booleans become Yes/No rather than `true`/`false`: a stat tile is read by a
 * person, and the raw literals are the programmer's spelling of the answer.
 */
export function formatReading(value: unknown): string {
  if (value === undefined || value === null) return "—";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "number") {
    return Number.isInteger(value) ? String(value) : value.toFixed(2);
  }
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}

const BYTE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"] as const;

/** Scales an exact byte count for display while leaving the API value intact. */
export function formatByteReading(value: number): { text: string; unit: string } {
  if (!Number.isFinite(value)) return { text: "—", unit: "B" };
  if (value === 0) return { text: "0", unit: "B" };

  const magnitude = Math.abs(value);
  const exponent = Math.min(
    Math.floor(Math.log(magnitude) / Math.log(1024)),
    BYTE_UNITS.length - 1,
  );
  const scaled = value / 1024 ** exponent;
  const maximumFractionDigits = Math.abs(scaled) >= 100 ? 0 : Math.abs(scaled) >= 10 ? 1 : 2;

  return {
    text: new Intl.NumberFormat(undefined, { maximumFractionDigits }).format(scaled),
    unit: BYTE_UNITS[exponent],
  };
}
