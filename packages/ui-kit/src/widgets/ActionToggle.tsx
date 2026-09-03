import * as React from "react";

import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import { cn } from "@loom/ui-kit/lib/utils";
import {
  configOptionalBoolean,
  configString,
  type ActionWidgetProps,
} from "@loom/ui-kit/widgets/types";

export function ActionToggleSkeleton({ className }: { className?: string }) {
  return <div className={cn("flex items-center justify-between gap-3", className)}><Skeleton className="h-4 w-24" /><Skeleton className="h-6 w-11 rounded-full" /></div>;
}

/**
 * A switch that runs an action with a boolean.
 *
 * **Optimistic, and it reverts.** The switch moves the instant it is pressed,
 * because waiting for a round trip makes a toggle feel broken; if the action
 * then fails, the switch snaps back to where it was. The caller's `onExecute`
 * rejection is what drives that, which is why it must reject rather than
 * swallow — a toggle that stayed on after a failed request would be lying about
 * the state of a service.
 *
 * A binding may opt into a reported starting/current state with
 * `config.stateDataPointId`. `renderWidget` resolves that boolean reading into
 * `config.currentValue`; stale status frames do not undo an optimistic press,
 * while the next genuinely changed reading reconciles the switch.
 *
 * The parameter name defaults to `value`, matching the widget contract, and can
 * be overridden per binding with `config.paramName` for an action that spells
 * it differently.
 */
export function ActionToggleWidget({
  label,
  actionId,
  description,
  config,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const reportedChecked = configOptionalBoolean(config, "currentValue");
  const [checked, setChecked] = React.useState(reportedChecked ?? false);
  const [pending, setPending] = React.useState(false);
  const lastAppliedReport = React.useRef(reportedChecked);
  const paramName = configString(config, "paramName", "value");
  const id = React.useId();

  React.useEffect(() => {
    if (pending || reportedChecked === undefined || reportedChecked === lastAppliedReport.current) {
      return;
    }
    lastAppliedReport.current = reportedChecked;
    setChecked(reportedChecked);
  }, [pending, reportedChecked]);

  async function change(next: boolean) {
    const previous = checked;
    setChecked(next);
    setPending(true);
    try {
      await onExecute(actionId, { [paramName]: next });
    } catch {
      setChecked(previous);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={cn("flex min-w-0 items-center justify-between gap-3", className)}>
      <Label htmlFor={id} className="min-w-0 truncate text-sm" title={description ?? label}>
        {label}
      </Label>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled || pending}
        onCheckedChange={(next) => void change(next)}
      />
    </div>
  );
}
