import * as React from "react";

import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { cn } from "@loom/ui-kit/lib/utils";
import {
  configString,
  configStringArray,
  type ActionWidgetProps,
} from "@loom/ui-kit/widgets/types";

/**
 * A dropdown that runs an action with one of a fixed set of strings.
 *
 * **The options come from the binding, not from the schema.** They are
 * `config.options`, typed into the binding editor when the widget is placed —
 * not read from the action's `paramsSchema.enum`. That is a deliberate
 * simplification and the same one `SchemaForm` already makes: neither
 * understands `enum` yet, and having the selector read it while the generated
 * parameter form ignored it would mean the same action offered a picker in one
 * place and a free-text box in the other. When `enum` support lands it belongs
 * in `SchemaForm` first, and this widget should then prefer the schema and keep
 * `config.options` as the override.
 *
 * A binding with no options renders a disabled trigger saying so, rather than
 * an empty menu that looks like a loading state.
 */
export function ActionSelectorWidget({
  label,
  actionId,
  description,
  config,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const options = configStringArray(config, "options");
  const paramName = configString(config, "paramName", "value");
  // "" rather than undefined so the Select is controlled from the first
  // render; Radix shows the placeholder for an empty value and only forbids it
  // on the items themselves.
  const [selected, setSelected] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const id = React.useId();

  async function choose(next: string) {
    const previous = selected;
    setSelected(next);
    setPending(true);
    try {
      await onExecute(actionId, { [paramName]: next });
    } catch {
      setSelected(previous);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-1", className)}>
      <Label htmlFor={id} className="truncate text-xs text-muted-foreground" title={description ?? label}>
        {label}
      </Label>
      <Select
        value={selected}
        disabled={disabled || pending || options.length === 0}
        onValueChange={(next) => void choose(next)}
      >
        <SelectTrigger id={id} aria-label={label}>
          <SelectValue placeholder={options.length === 0 ? "No options configured" : "Choose…"} />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
