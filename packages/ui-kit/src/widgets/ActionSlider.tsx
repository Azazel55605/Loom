import * as React from "react";

import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Slider } from "@loom/ui-kit/components/ui/slider";
import { cn } from "@loom/ui-kit/lib/utils";
import {
  configNumber,
  configOptionalNumber,
  configString,
  type ActionWidgetProps,
} from "@loom/ui-kit/widgets/types";

export function ActionSliderSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><div className="flex justify-between"><Skeleton className="h-3 w-20" /><Skeleton className="h-4 w-6" /></div><Skeleton className="h-4 w-full rounded-full" /></div>;
}

/**
 * A slider that runs an action with a number.
 *
 * **Fires on release, never while dragging.** A drag emits a value per frame,
 * and sending each one would be dozens of requests to a service for one
 * gesture — with the added hazard that they can arrive out of order and leave
 * the service on a value the user passed through rather than the one they
 * stopped on. Radix separates the two callbacks for exactly this, so the thumb
 * tracks the drag locally and only `onValueCommit` reaches the connector.
 *
 * Bounds come from the binding: `config.min`, `config.max`, `config.step`.
 * When a binding supplies `config.linkedDataPointId`, `renderWidget` resolves
 * that reading into `config.currentValue`; the slider follows live reports
 * whenever it is not being dragged or waiting for its committed action.
 * Bindings without that link retain the original fire-and-forget behaviour.
 */
export function ActionSliderWidget({
  label,
  actionId,
  description,
  config,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const min = configNumber(config, "min", 0);
  const max = configNumber(config, "max", 100);
  const step = configNumber(config, "step", 1);
  const paramName = configString(config, "paramName", "value");
  const reportedValue = configOptionalNumber(config, "currentValue");
  const normalizedReport =
    reportedValue === undefined ? undefined : Math.min(max, Math.max(min, reportedValue));

  const [value, setValue] = React.useState(normalizedReport ?? min);
  const [pending, setPending] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);
  const committed = React.useRef(normalizedReport ?? min);
  const lastAppliedReport = React.useRef(normalizedReport);
  const id = React.useId();

  React.useEffect(() => {
    if (
      dragging ||
      pending ||
      normalizedReport === undefined ||
      normalizedReport === lastAppliedReport.current
    ) {
      return;
    }
    lastAppliedReport.current = normalizedReport;
    committed.current = normalizedReport;
    setValue(normalizedReport);
  }, [dragging, normalizedReport, pending]);

  async function commit(next: number) {
    const previous = committed.current;
    committed.current = next;
    lastAppliedReport.current = normalizedReport;
    setDragging(false);
    setPending(true);
    try {
      await onExecute(actionId, { [paramName]: next });
    } catch {
      committed.current = previous;
      setValue(previous);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-2", className)}>
      <div className="flex items-baseline justify-between gap-2">
        <Label htmlFor={id} className="min-w-0 truncate text-xs text-muted-foreground" title={description ?? label}>
          {label}
        </Label>
        <span className="shrink-0 text-sm font-medium tabular-nums">{value}</span>
      </div>
      <Slider
        id={id}
        min={min}
        max={max}
        step={step}
        value={[value]}
        disabled={disabled || pending}
        aria-label={label}
        onValueChange={([next]) => {
          setDragging(true);
          setValue(next ?? min);
        }}
        onValueCommit={([next]) => void commit(next ?? min)}
      />
    </div>
  );
}
