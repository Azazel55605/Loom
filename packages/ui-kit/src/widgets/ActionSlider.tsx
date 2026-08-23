import * as React from "react";

import { Label } from "@loom/ui-kit/components/ui/label";
import { Slider } from "@loom/ui-kit/components/ui/slider";
import { cn } from "@loom/ui-kit/lib/utils";
import { configNumber, configString, type ActionWidgetProps } from "@loom/ui-kit/widgets/types";

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
 * Bounds come from the binding: `config.min`, `config.max`, `config.step`. Like
 * `Toggle`, this shows the value it last sent, not the service's current one —
 * see that widget's note.
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

  const [value, setValue] = React.useState(min);
  const [pending, setPending] = React.useState(false);
  const committed = React.useRef(min);
  const id = React.useId();

  async function commit(next: number) {
    const previous = committed.current;
    committed.current = next;
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
        onValueChange={([next]) => setValue(next ?? min)}
        onValueCommit={([next]) => void commit(next ?? min)}
      />
    </div>
  );
}
