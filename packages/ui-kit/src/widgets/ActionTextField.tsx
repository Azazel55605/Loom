import * as React from "react";
import { Loader2, SendHorizontal } from "lucide-react";

import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { configString, type ActionWidgetProps } from "@loom/ui-kit/widgets/types";

export function ActionTextFieldSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><Skeleton className="h-3 w-20" /><div className="flex gap-2"><Skeleton className="h-9 flex-1" /><Skeleton className="h-9 w-9" /></div></div>;
}

/**
 * A text input that runs an action with a string.
 *
 * A real `<form>` rather than an input with a keydown handler, so Enter submits
 * the way it does everywhere else in the browser and the submit button needs no
 * separate wiring. `noValidate` for the same reason as the rest of the app: a
 * native validation bubble is a browser-default control, and it would pre-empt
 * the connector's own, better-worded `invalidParams` message.
 *
 * The field clears on success and keeps its text on failure, so a rejected
 * value can be corrected rather than retyped.
 */
export function ActionTextFieldWidget({
  label,
  actionId,
  description,
  config,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const [value, setValue] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const paramName = configString(config, "paramName", "value");
  const placeholder = configString(config, "placeholder", "");
  const id = React.useId();

  async function submit() {
    if (value.trim().length === 0) return;
    setPending(true);
    try {
      await onExecute(actionId, { [paramName]: value });
      setValue("");
    } catch {
      // Left as typed, so the value can be corrected. The caller toasts.
    } finally {
      setPending(false);
    }
  }

  return (
    <form
      className={cn("flex min-w-0 flex-col justify-center gap-1", className)}
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <Label htmlFor={id} className="truncate text-xs text-muted-foreground" title={description ?? label}>
        {label}
      </Label>
      <div className="flex min-w-0 items-center gap-2">
        <Input
          id={id}
          value={value}
          placeholder={placeholder.length > 0 ? placeholder : undefined}
          disabled={disabled || pending}
          onChange={(event) => setValue(event.target.value)}
        />
        <Button
          type="submit"
          size="icon"
          variant="outline"
          className="h-9 w-9 shrink-0"
          aria-label={`Run ${label}`}
          disabled={disabled || pending || value.trim().length === 0}
        >
          {pending ? (
            <Loader2 className="animate-spin" aria-hidden="true" />
          ) : (
            <SendHorizontal aria-hidden="true" />
          )}
        </Button>
      </div>
    </form>
  );
}
