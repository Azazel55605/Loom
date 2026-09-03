import * as React from "react";
import { Plus, Trash2, X } from "lucide-react";

import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import { cn } from "@loom/ui-kit/lib/utils";
import { matchesTarget } from "@loom/ui-kit/lib/connector-details";
import type {
  ActionWidgetType,
  ChartType,
  ConnectorAction,
  DataPointDescriptor,
  WidgetBinding,
} from "@loom/ui-kit/lib/api";
import {
  describeActionWidget,
  describeDisplayWidget,
  displayChartType,
  displayWidgetFromKey,
  displayWidgetKey,
  getCompatibleActionWidgetTypes,
  getCompatibleWidgetTypes,
  type DisplayWidgetKey,
} from "@loom/ui-kit/widgets/compatibility";

const CHART_TYPES: { value: ChartType; label: string }[] = [
  { value: "line", label: "Line" },
  { value: "bar", label: "Bar" },
  { value: "pie", label: "Pie" },
];

function asObject(config: unknown): Record<string, unknown> {
  return typeof config === "object" && config !== null && !Array.isArray(config)
    ? { ...(config as Record<string, unknown>) }
    : {};
}

function numberField(config: Record<string, unknown>, key: string): string {
  const value = config[key];
  return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
}

/**
 * Builds the `widgetBindings` array for one placement.
 *
 * Shared by both flows that produce one: the add-placement dialog seeds it from
 * the connector's `defaultLayout` and this edits it before creation; the
 * edit-bindings dialog on a placed card loads the stored array and this edits
 * that. One editor rather than two, because the two flows differ only in where
 * the starting array came from.
 *
 * ## Why the widget list is filtered
 *
 * A binding whose widget cannot draw its data renders as a blank space with no
 * explanation — a `statusDot` over a time series has nothing to colour. So the
 * widget picker only offers what the chosen data point's `valueType` supports
 * (`getCompatibleWidgetTypes`), and for an action only what its `paramsSchema`
 * suits (`getCompatibleActionWidgetTypes`). Changing the target re-picks the
 * widget when the current one no longer fits, rather than leaving a selection
 * that would silently produce nothing.
 *
 * This is guidance, not enforcement. The backend validates that the ids *exist*
 * — see `docs/adr/0014-widget-binding-model.md` — and deliberately says nothing
 * about whether a widget suits its data, so a binding written straight against
 * the API can still be nonsense.
 */
export function PlacementBindingEditor({
  dataPoints,
  actions,
  targetId,
  value,
  onChange,
  disabled,
  className,
}: {
  dataPoints: DataPointDescriptor[];
  actions: ConnectorAction[];
  /** The placement view whose descriptors may be bound. */
  targetId: string | null;
  value: WidgetBinding[];
  onChange: (next: WidgetBinding[]) => void;
  disabled?: boolean;
  className?: string;
}) {
  const availableDataPoints = dataPoints.filter((point) => matchesTarget(point, targetId));
  const availableActions = actions.filter((action) => matchesTarget(action, targetId));
  // Stable identity per row, so removing one does not shift every row below it
  // onto a different React key. Radix `Select` keeps internal state keyed by
  // component identity, and a reused instance whose controlled value is no
  // longer among its items clears itself and reports `""` — which is how a
  // perfectly good binding ended up submitting an empty widget type. Bindings
  // carry no id of their own, so one is minted here and travels with the row.
  const rowKeys = React.useRef<number[]>([]);
  const nextRowKey = React.useRef(0);
  while (rowKeys.current.length < value.length) {
    rowKeys.current.push(nextRowKey.current++);
  }
  rowKeys.current.length = value.length;

  function replace(index: number, binding: WidgetBinding) {
    onChange(value.map((existing, position) => (position === index ? binding : existing)));
  }

  function removeAt(index: number) {
    rowKeys.current.splice(index, 1);
    onChange(value.filter((_, position) => position !== index));
  }

  function addBinding() {
    if (availableDataPoints.length > 0) {
      const point = availableDataPoints[0];
      const [widget] = getCompatibleWidgetTypes(point.valueType);
      onChange([
        ...value,
        {
          display: {
            dataPointId: point.id,
            widgetType: displayWidgetFromKey(widget),
            config: {},
          },
        },
      ]);
      return;
    }
    if (availableActions.length > 0) {
      const action = availableActions[0];
      onChange([
        ...value,
        {
          action: {
            actionId: action.id,
            widgetType: getCompatibleActionWidgetTypes(action)[0],
            config: {},
          },
        },
      ]);
    }
  }

  const canAdd = availableDataPoints.length > 0 || availableActions.length > 0;

  return (
    <div className={cn("flex flex-col gap-3", className)}>
      {value.length === 0 ? (
        <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
          No widgets yet. This placement would show its connector's name and health and nothing
          else.
        </p>
      ) : (
        <ul className="flex flex-col gap-3">
          {value.map((binding, index) => (
            <li key={rowKeys.current[index]} className="surface-panel rounded-lg border p-3">
              <BindingRow
                binding={binding}
                dataPoints={availableDataPoints}
                actions={availableActions}
                disabled={disabled}
                idPrefix={`binding-${index}`}
                onChange={(next) => replace(index, next)}
                onRemove={() => removeAt(index)}
              />
            </li>
          ))}
        </ul>
      )}

      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled || !canAdd}
          onClick={addBinding}
        >
          <Plus aria-hidden="true" />
          Add widget
        </Button>
        {!canAdd ? (
          <p className="mt-2 text-xs text-muted-foreground">
            This connector declares no data points and no actions, so there is nothing to bind.
          </p>
        ) : null}
      </div>
    </div>
  );
}

function BindingRow({
  binding,
  dataPoints,
  actions,
  disabled,
  idPrefix,
  onChange,
  onRemove,
}: {
  binding: WidgetBinding;
  dataPoints: DataPointDescriptor[];
  actions: ConnectorAction[];
  disabled?: boolean;
  idPrefix: string;
  onChange: (next: WidgetBinding) => void;
  onRemove: () => void;
}) {
  const kind = "display" in binding ? "display" : "action";

  function switchKind(next: "display" | "action") {
    if (next === kind) return;
    if (next === "display") {
      const point = dataPoints[0];
      if (point === undefined) return;
      onChange({
        display: {
          dataPointId: point.id,
          widgetType: displayWidgetFromKey(getCompatibleWidgetTypes(point.valueType)[0]),
          config: {},
        },
      });
      return;
    }
    const action = actions[0];
    if (action === undefined) return;
    onChange({
      action: {
        actionId: action.id,
        widgetType: getCompatibleActionWidgetTypes(action)[0],
        config: {},
      },
    });
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start justify-between gap-3">
        <SegmentedControl
          label="Binding kind"
          value={kind}
          onChange={switchKind}
          options={[
            { value: "display" as const, label: "Display" },
            { value: "action" as const, label: "Action" },
          ]}
          // Switching kinds needs somewhere to switch to. A connector with no
          // actions cannot host an action binding, so the control is disabled
          // rather than offering a choice that would silently do nothing.
          className={
            disabled || (dataPoints.length === 0 && actions.length === 0)
              ? "pointer-events-none opacity-50"
              : undefined
          }
        />
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-8 w-8 shrink-0 text-muted-foreground hover:text-destructive"
          aria-label="Remove widget"
          disabled={disabled}
          onClick={onRemove}
        >
          <Trash2 aria-hidden="true" />
        </Button>
      </div>

      {"display" in binding ? (
        <DisplayBindingFields
          binding={binding.display}
          dataPoints={dataPoints}
          disabled={disabled}
          idPrefix={idPrefix}
          onChange={(display) => onChange({ display })}
        />
      ) : (
        <ActionBindingFields
          binding={binding.action}
          actions={actions}
          disabled={disabled}
          idPrefix={idPrefix}
          onChange={(action) => onChange({ action })}
        />
      )}
    </div>
  );
}

type DisplayBinding = Extract<WidgetBinding, { display: unknown }>["display"];
type ActionBinding = Extract<WidgetBinding, { action: unknown }>["action"];

function DisplayBindingFields({
  binding,
  dataPoints,
  disabled,
  idPrefix,
  onChange,
}: {
  binding: DisplayBinding;
  dataPoints: DataPointDescriptor[];
  disabled?: boolean;
  idPrefix: string;
  onChange: (next: DisplayBinding) => void;
}) {
  const descriptor = dataPoints.find((point) => point.id === binding.dataPointId);
  const compatible = descriptor === undefined ? [] : getCompatibleWidgetTypes(descriptor.valueType);
  const currentKey = displayWidgetKey(binding.widgetType);
  const chartType = displayChartType(binding.widgetType) ?? "line";
  const config = asObject(binding.config);

  function setConfigNumber(key: string, raw: string) {
    const next = { ...config };
    if (raw.trim() === "") {
      delete next[key];
    } else {
      const parsed = Number(raw);
      // A half-typed "-" or "1e" is not a number yet. Dropping the key rather
      // than storing NaN keeps the binding serializable at every keystroke —
      // the same rule SchemaForm follows.
      if (Number.isFinite(parsed)) next[key] = parsed;
      else delete next[key];
    }
    onChange({ ...binding, config: next });
  }

  function setBarOrientation(orientation: "vertical" | "horizontal") {
    const next = { ...config };
    // Vertical bars are the backwards-compatible default, so old bindings and
    // bindings switched back to the default need no extra persisted field.
    if (orientation === "vertical") delete next.orientation;
    else next.orientation = orientation;
    onChange({ ...binding, config: next });
  }

  const bounded = currentKey === "gauge" || currentKey === "progressBar";

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <Field id={`${idPrefix}-point`} label="Data point">
        <Select
          value={binding.dataPointId}
          disabled={disabled || dataPoints.length === 0}
          onValueChange={(nextId) => {
            // Radix reports "" when it clears its own selection; no data point
            // is ever named that, so it is never a real choice.
            if (nextId === "") return;
            const point = dataPoints.find((candidate) => candidate.id === nextId);
            if (point === undefined) return;
            const allowed = getCompatibleWidgetTypes(point.valueType);
            // Keep the chosen widget when the new data point still supports it;
            // otherwise fall to that type's first suitable widget rather than
            // leaving a pairing that renders nothing.
            const key = allowed.includes(currentKey) ? currentKey : allowed[0];
            onChange({
              ...binding,
              dataPointId: nextId,
              widgetType: displayWidgetFromKey(key, chartType),
            });
          }}
        >
          <SelectTrigger id={`${idPrefix}-point`}>
            <SelectValue placeholder="Choose a data point" />
          </SelectTrigger>
          <SelectContent>
            {dataPoints.map((point) => (
              <SelectItem key={point.id} value={point.id}>
                {point.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {descriptor === undefined ? (
          <p className="text-xs text-destructive">
            This connector no longer declares <code className="font-mono">{binding.dataPointId}</code>.
          </p>
        ) : null}
      </Field>

      <Field id={`${idPrefix}-widget`} label="Widget">
        <Select
          value={currentKey}
          disabled={disabled || compatible.length === 0}
          onValueChange={(next) => {
            if (next === "") return;
            onChange({
              ...binding,
              widgetType: displayWidgetFromKey(next as DisplayWidgetKey, chartType),
            });
          }}
        >
          <SelectTrigger id={`${idPrefix}-widget`}>
            <SelectValue placeholder="Choose a widget" />
          </SelectTrigger>
          <SelectContent>
            {compatible.map((key) => (
              <SelectItem key={key} value={key}>
                {describeDisplayWidget(key)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      {currentKey === "metricChart" ? (
        <Field id={`${idPrefix}-chart`} label="Chart type">
          <Select
            value={chartType}
            disabled={disabled}
            onValueChange={(next) => {
              if (next === "") return;
              onChange({
                ...binding,
                widgetType: displayWidgetFromKey("metricChart", next as ChartType),
              });
            }}
          >
            <SelectTrigger id={`${idPrefix}-chart`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CHART_TYPES.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
      ) : null}

      {currentKey === "metricChart" && chartType === "bar" ? (
        <Field id={`${idPrefix}-orientation`} label="Bar orientation">
          <Select
            value={config.orientation === "horizontal" ? "horizontal" : "vertical"}
            disabled={disabled}
            onValueChange={(next) => {
              if (next === "vertical" || next === "horizontal") setBarOrientation(next);
            }}
          >
            <SelectTrigger id={`${idPrefix}-orientation`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="vertical">Vertical bars</SelectItem>
              <SelectItem value="horizontal">Horizontal bars</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      ) : null}

      {bounded ? (
        <>
          <Field id={`${idPrefix}-min`} label="Minimum">
            <Input
              id={`${idPrefix}-min`}
              inputMode="decimal"
              placeholder="0"
              disabled={disabled}
              value={numberField(config, "min")}
              onChange={(event) => setConfigNumber("min", event.target.value)}
            />
          </Field>
          <Field id={`${idPrefix}-max`} label="Maximum">
            <Input
              id={`${idPrefix}-max`}
              inputMode="decimal"
              placeholder="100"
              disabled={disabled}
              value={numberField(config, "max")}
              onChange={(event) => setConfigNumber("max", event.target.value)}
            />
          </Field>
        </>
      ) : null}
    </div>
  );
}

function ActionBindingFields({
  binding,
  actions,
  disabled,
  idPrefix,
  onChange,
}: {
  binding: ActionBinding;
  actions: ConnectorAction[];
  disabled?: boolean;
  idPrefix: string;
  onChange: (next: ActionBinding) => void;
}) {
  const action = actions.find((candidate) => candidate.id === binding.actionId);
  const compatible = action === undefined ? [] : getCompatibleActionWidgetTypes(action);
  const config = asObject(binding.config);
  const options = Array.isArray(config.options)
    ? config.options.filter((entry): entry is string => typeof entry === "string")
    : [];

  function setConfig(key: string, next: unknown) {
    const merged = { ...config };
    if (next === undefined) delete merged[key];
    else merged[key] = next;
    onChange({ ...binding, config: merged });
  }

  function setConfigNumber(key: string, raw: string) {
    if (raw.trim() === "") return setConfig(key, undefined);
    const parsed = Number(raw);
    setConfig(key, Number.isFinite(parsed) ? parsed : undefined);
  }

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <Field id={`${idPrefix}-action`} label="Action">
        <Select
          value={binding.actionId}
          disabled={disabled || actions.length === 0}
          onValueChange={(nextId) => {
            if (nextId === "") return;
            const next = actions.find((candidate) => candidate.id === nextId);
            if (next === undefined) return;
            const allowed = getCompatibleActionWidgetTypes(next);
            const widgetType = allowed.includes(binding.widgetType)
              ? binding.widgetType
              : allowed[0];
            onChange({ ...binding, actionId: nextId, widgetType, config: {} });
          }}
        >
          <SelectTrigger id={`${idPrefix}-action`}>
            <SelectValue placeholder="Choose an action" />
          </SelectTrigger>
          <SelectContent>
            {actions.map((candidate) => (
              <SelectItem key={candidate.id} value={candidate.id}>
                {candidate.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {action === undefined ? (
          <p className="text-xs text-destructive">
            This connector no longer offers <code className="font-mono">{binding.actionId}</code>.
          </p>
        ) : null}
      </Field>

      <Field id={`${idPrefix}-control`} label="Control">
        <Select
          value={binding.widgetType}
          disabled={disabled || compatible.length === 0}
          onValueChange={(next) => {
            if (next === "") return;
            onChange({ ...binding, widgetType: next as ActionWidgetType });
          }}
        >
          <SelectTrigger id={`${idPrefix}-control`}>
            <SelectValue placeholder="Choose a control" />
          </SelectTrigger>
          <SelectContent>
            {compatible.map((widgetType) => (
              <SelectItem key={widgetType} value={widgetType}>
                {describeActionWidget(widgetType)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>

      {binding.widgetType === "slider" ? (
        <>
          <Field id={`${idPrefix}-min`} label="Minimum">
            <Input
              id={`${idPrefix}-min`}
              inputMode="decimal"
              placeholder="0"
              disabled={disabled}
              value={numberField(config, "min")}
              onChange={(event) => setConfigNumber("min", event.target.value)}
            />
          </Field>
          <Field id={`${idPrefix}-max`} label="Maximum">
            <Input
              id={`${idPrefix}-max`}
              inputMode="decimal"
              placeholder="100"
              disabled={disabled}
              value={numberField(config, "max")}
              onChange={(event) => setConfigNumber("max", event.target.value)}
            />
          </Field>
          <Field id={`${idPrefix}-step`} label="Step">
            <Input
              id={`${idPrefix}-step`}
              inputMode="decimal"
              placeholder="1"
              disabled={disabled}
              value={numberField(config, "step")}
              onChange={(event) => setConfigNumber("step", event.target.value)}
            />
          </Field>
        </>
      ) : null}

      {binding.widgetType === "selector" ? (
        <div className="sm:col-span-2">
          <OptionsEditor
            options={options}
            disabled={disabled}
            idPrefix={idPrefix}
            onChange={(next) => setConfig("options", next.length > 0 ? next : undefined)}
          />
        </div>
      ) : null}
    </div>
  );
}

/**
 * The `config.options` list for a `selector` binding.
 *
 * Typed in here rather than read from the action's schema — see
 * `ActionSelectorWidget` for why that is a deliberate simplification and what
 * would change it.
 */
function OptionsEditor({
  options,
  disabled,
  idPrefix,
  onChange,
}: {
  options: string[];
  disabled?: boolean;
  idPrefix: string;
  onChange: (next: string[]) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label>Options</Label>
      {options.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          A dropdown with no options is disabled on the dashboard. Add the values this action
          accepts.
        </p>
      ) : null}
      {options.map((option, index) => (
        <div key={index} className="flex items-center gap-2">
          <Input
            id={`${idPrefix}-option-${index}`}
            aria-label={`Option ${index + 1}`}
            value={option}
            disabled={disabled}
            onChange={(event) =>
              onChange(
                options.map((existing, position) =>
                  position === index ? event.target.value : existing,
                ),
              )
            }
          />
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-9 w-9 shrink-0 text-muted-foreground hover:text-destructive"
            aria-label={`Remove option ${index + 1}`}
            disabled={disabled}
            onClick={() => onChange(options.filter((_, position) => position !== index))}
          >
            <X aria-hidden="true" />
          </Button>
        </div>
      ))}
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => onChange([...options, ""])}
        >
          <Plus aria-hidden="true" />
          Add option
        </Button>
      </div>
    </div>
  );
}

function Field({
  id,
  label,
  children,
}: {
  id: string;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
    </div>
  );
}
