import * as React from "react";

import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import { cn } from "@loom/ui-kit/lib/utils";
import { configString, type ActionWidgetProps } from "@loom/ui-kit/widgets/types";

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
 * **It shows what this widget last set, not what the service is.** An action
 * binding names an action and nothing else — that is the whole point of the
 * `Display`/`Action` split — so there is no data point behind this switch to
 * read a starting position from, and it begins unchecked on every mount. To see
 * the real state, bind a `StatusDot` to the connector's boolean data point
 * alongside it. Closing that gap means letting an action binding reference a
 * data point, which is a connector-contract change and deliberately not
 * invented here.
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
  const [checked, setChecked] = React.useState(false);
  const [pending, setPending] = React.useState(false);
  const paramName = configString(config, "paramName", "value");
  const id = React.useId();

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
