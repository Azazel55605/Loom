import { cn } from "@loom/ui-kit/lib/utils";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  configStringMap,
  formatReading,
  type DisplayWidgetProps,
} from "@loom/ui-kit/widgets/types";

export function StatusDotSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><div className="flex items-center gap-2"><Skeleton className="h-3 w-3 rounded-full" /><Skeleton className="h-4 w-20" /></div><Skeleton className="h-3 w-14" /></div>;
}

/** The tokens a `config.colorMap` entry may name. Deliberately the status
 *  palette and nothing else: a dot is a status indicator, and letting a binding
 *  supply an arbitrary colour would put a fifth meaning-carrying hue on a
 *  dashboard that already has four. */
const DOT_TOKENS: Record<string, string> = {
  healthy: "bg-status-healthy",
  degraded: "bg-status-degraded",
  down: "bg-status-down",
  unknown: "bg-status-unknown",
};

const NEUTRAL = "bg-status-unknown";

function dotClass(value: unknown, config: unknown): string {
  if (typeof value === "boolean") {
    // The two-colour case, and the only one that needs no configuration: a
    // boolean data point means on or off, and "off" is not a fault, so the
    // false side is the neutral token rather than the down one.
    return value ? DOT_TOKENS.healthy : NEUTRAL;
  }

  if (typeof value === "string") {
    // Multi-state strings map through `config.colorMap`, e.g.
    // `{ "running": "healthy", "restarting": "degraded", "exited": "down" }`.
    // Without a map there is no way to know which of a connector's own state
    // names is the good one, so an unmapped value stays neutral rather than
    // being guessed at.
    const mapped = configStringMap(config, "colorMap")[value];
    return mapped !== undefined ? (DOT_TOKENS[mapped] ?? NEUTRAL) : NEUTRAL;
  }

  return NEUTRAL;
}

/**
 * A coloured dot for a state that is not a number.
 *
 * Never colour alone: the dot always sits beside the formatted value, because
 * colour is exactly the channel a colour-blind user does not have. The dot is
 * the fast read and the text is the actual answer.
 */
export function StatusDotWidget({ label, value, config, className }: DisplayWidgetProps) {
  const text = formatReading(value);

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-1", className)}>
      <div className="flex min-w-0 items-center gap-2">
        <span
          aria-hidden="true"
          className={cn(
            "h-2.5 w-2.5 shrink-0 rounded-full transition-colors",
            dotClass(value, config),
          )}
        />
        <span className="min-w-0 whitespace-normal text-sm font-medium [overflow-wrap:anywhere]">
          {text}
        </span>
      </div>
      <span className="whitespace-normal text-xs text-muted-foreground [overflow-wrap:anywhere]">
        {label}
      </span>
    </div>
  );
}
