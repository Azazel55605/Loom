import * as React from "react";
import { Loader2 } from "lucide-react";

import {
  ActionParamsDialog,
  takesParameters,
} from "@loom/ui-kit/components/ActionParamsDialog";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import type { ConnectorAction } from "@loom/ui-kit/lib/api";
import type { ActionWidgetProps } from "@loom/ui-kit/widgets/types";

export function ActionButtonSkeleton({ className }: { className?: string }) {
  return <Skeleton className={cn("h-9 w-full", className)} />;
}

/**
 * A control that runs one action.
 *
 * The general case, and the fallback every action is compatible with: whatever
 * an action's parameters are, a button plus a generated form can collect them.
 * That form is `ActionParamsDialog`, the same one `ConnectorCard` uses — a
 * parameterless action fires straight away, anything else opens the dialog.
 * Reusing it is the point: a second parameter prompt built for widgets would
 * drift from the card's, and only one of them would get fixed.
 */
export function ActionButtonWidget({
  label,
  actionId,
  paramsSchema,
  description,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const [pending, setPending] = React.useState(false);
  const [promptOpen, setPromptOpen] = React.useState(false);

  // `ActionParamsDialog` speaks in whole actions rather than loose fields, so
  // the descriptor is reassembled here from what the binding was given.
  const action: ConnectorAction = {
    id: actionId,
    label,
    description: description ?? null,
    paramsSchema: paramsSchema ?? {},
  };

  async function run(params: Record<string, unknown>) {
    setPending(true);
    try {
      await onExecute(actionId, params);
      setPromptOpen(false);
    } catch {
      // The caller toasts. Swallowed here so a rejected promise does not reach
      // the console as an unhandled error on every declined action.
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={cn("flex min-w-0 items-center", className)}>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="w-full"
        disabled={disabled || pending}
        title={description ?? undefined}
        onClick={() => {
          if (takesParameters(action)) {
            setPromptOpen(true);
            return;
          }
          void run({});
        }}
      >
        {pending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}
        <span className="truncate">{label}</span>
      </Button>

      <ActionParamsDialog
        action={promptOpen ? action : null}
        connectorName={label}
        isPending={pending}
        onOpenChange={(open) => {
          if (!open) setPromptOpen(false);
        }}
        onSubmit={(params) => void run(params)}
      />
    </div>
  );
}
