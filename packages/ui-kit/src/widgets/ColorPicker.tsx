import * as React from "react";
import { Check, Palette } from "lucide-react";

import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@loom/ui-kit/components/ui/popover";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import {
  configString,
  type ActionWidgetProps,
  type DisplayWidgetProps,
} from "@loom/ui-kit/widgets/types";

const COLOR_PRESETS = [
  "#EF4444",
  "#F97316",
  "#EAB308",
  "#22C55E",
  "#06B6D4",
  "#3B82F6",
  "#8B5CF6",
  "#EC4899",
] as const;

/** The widget boundary accepts one colour representation: `#RRGGBB`. */
export function normalizeHexColor(value: unknown): string | null {
  if (typeof value !== "string" || !/^#[0-9a-fA-F]{6}$/.test(value)) return null;
  return value.toUpperCase();
}

export function ColorPickerSkeleton({ className }: { className?: string }) {
  return (
    <div className={cn("flex min-w-0 items-center gap-3", className)}>
      <Skeleton className="size-10 shrink-0 rounded-md" />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="h-3 w-16" />
      </div>
    </div>
  );
}

/** Read-only colour reading, retaining the exact interoperable hex value. */
export function ColorPickerDisplayWidget({
  label,
  value,
  className,
}: DisplayWidgetProps) {
  const color = normalizeHexColor(value);

  return (
    <div className={cn("flex min-w-0 items-center gap-3", className)}>
      <ColorSwatch color={color} />
      <div className="min-w-0">
        <p className="break-words text-sm font-medium">{color ?? "—"}</p>
        <p className="break-words text-xs text-muted-foreground">{label}</p>
      </div>
    </div>
  );
}

/**
 * A compact, themed colour control without the platform-native colour input.
 *
 * `config.currentValue` is injected by `renderWidget` from the binding's
 * optional `linkedDataPointId`. Reports reconcile while the popover is closed;
 * an in-progress edit and a pending optimistic update are never overwritten.
 */
export function ColorPickerActionWidget({
  label,
  actionId,
  description,
  config,
  onExecute,
  disabled,
  className,
}: ActionWidgetProps) {
  const reportedColor = normalizeHexColor(configString(config, "currentValue", ""));
  const paramName = configString(config, "paramName", "value");
  const [color, setColor] = React.useState(reportedColor ?? "#000000");
  const [draft, setDraft] = React.useState(color);
  const [open, setOpen] = React.useState(false);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const lastAppliedReport = React.useRef(reportedColor);
  const id = React.useId();

  React.useEffect(() => {
    if (
      open ||
      pending ||
      reportedColor === null ||
      reportedColor === lastAppliedReport.current
    ) {
      return;
    }
    lastAppliedReport.current = reportedColor;
    setColor(reportedColor);
    setDraft(reportedColor);
  }, [open, pending, reportedColor]);

  async function apply() {
    const next = normalizeHexColor(draft);
    if (next === null) {
      setError("Enter a colour as #RRGGBB.");
      return;
    }

    const previous = color;
    lastAppliedReport.current = reportedColor;
    setColor(next);
    setPending(true);
    setError(null);
    try {
      await onExecute(actionId, { [paramName]: next });
      setDraft(next);
      setOpen(false);
    } catch {
      setColor(previous);
      setDraft(previous);
    } finally {
      setPending(false);
    }
  }

  return (
    <div className={cn("flex min-w-0 items-center justify-between gap-3", className)}>
      <Label className="min-w-0 break-words text-sm">
        {label}
      </Label>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (next) {
            setDraft(color);
            setError(null);
          }
        }}
      >
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            className="h-[var(--touch-target-size)] min-w-[var(--touch-target-size)] shrink-0 gap-2 px-3"
            disabled={disabled || pending}
            aria-label={`${label}: ${color}`}
          >
            <span
              aria-hidden="true"
              className="size-5 rounded-sm border border-border shadow-inner"
              style={{ backgroundColor: color }}
            />
            <span className="font-mono text-xs">{color}</span>
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-[min(20rem,calc(100vw-2rem))]">
          <form
            className="flex flex-col gap-4"
            onSubmit={(event) => {
              event.preventDefault();
              void apply();
            }}
          >
            <div className="flex flex-col gap-1">
              <p className="text-sm font-medium">Choose {label.toLocaleLowerCase()}</p>
              {description ? (
                <p className="text-xs text-muted-foreground">{description}</p>
              ) : null}
            </div>
            <div
              role="group"
              aria-label="Colour presets"
              className="grid grid-cols-4 gap-2"
            >
              {COLOR_PRESETS.map((preset) => {
                const selected = normalizeHexColor(draft) === preset;
                return (
                  <button
                    key={preset}
                    type="button"
                    aria-label={preset}
                    aria-pressed={selected}
                    className={cn(
                      "flex size-[var(--touch-target-size)] items-center justify-center rounded-md border border-border shadow-sm",
                      "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                      selected && "ring-2 ring-ring ring-offset-2 ring-offset-background",
                    )}
                    style={{ backgroundColor: preset }}
                    onClick={() => {
                      setDraft(preset);
                      setError(null);
                    }}
                  >
                    {selected ? (
                      <Check className="size-4 text-white drop-shadow" aria-hidden="true" />
                    ) : null}
                  </button>
                );
              })}
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor={id}>Hex colour</Label>
              <Input
                id={id}
                value={draft}
                placeholder="#2F7FED"
                autoCapitalize="characters"
                spellCheck={false}
                aria-invalid={error !== null}
                onChange={(event) => {
                  setDraft(event.target.value);
                  setError(null);
                }}
              />
              {error ? <p className="text-xs text-destructive">{error}</p> : null}
            </div>
            <Button type="submit" disabled={pending}>
              <Palette aria-hidden="true" />
              Apply colour
            </Button>
          </form>
        </PopoverContent>
      </Popover>
    </div>
  );
}

function ColorSwatch({ color }: { color: string | null }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "size-10 shrink-0 rounded-md border border-border shadow-inner",
        color === null && "bg-muted",
      )}
      style={color === null ? undefined : { backgroundColor: color }}
    />
  );
}
