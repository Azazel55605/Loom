import { Maximize2 } from "lucide-react";

import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { type DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

export function LogPreviewSkeleton({ className }: { className?: string }) {
  return <div className={cn("space-y-2", className)}><Skeleton className="h-3 w-20" /><Skeleton className="h-5 w-full" /></div>;
}

/**
 * The most recent log line, on one line, with a way to see the rest.
 *
 * The compact half of the log stream. A log is the one reading whose natural
 * size is unbounded, and a dashboard grid is the one place where that is
 * ruinous: a tile sized for a widget cannot also be sized for a thousand lines,
 * and letting it try is what stretched the grid around it. So the grid shows
 * the newest line — which is the part anyone glancing at a dashboard actually
 * wants — and the full pane lives one click away in the detail modal, where
 * there is room for it.
 *
 * Same value, same contract, same font as `LogStreamWidget`: a single `String`
 * data point, newline-separated. This is a second *rendering* of one binding,
 * not a second widget type, so nothing about a saved placement changes and
 * nobody has to choose between them.
 */
export function LogPreviewWidget({
  label,
  value,
  className,
  onExpand,
}: DisplayWidgetProps & {
  /** Opens the full log. Omitted where there is nowhere to open it — the
   *  affordance then goes away rather than sitting there doing nothing. */
  onExpand?: () => void;
}) {
  const text = typeof value === "string" ? value : "";
  // Trailing newlines are ordinary in a log buffer, so the last *line* and the
  // last non-empty line are rarely the same thing. Showing the former means
  // showing a blank tile for a stream that is working perfectly.
  const latest = text.split("\n").reverse().find((line) => line.trim().length > 0) ?? null;

  return (
    <div className={cn("flex min-w-0 flex-col justify-center gap-1", className)}>
      <div className="flex min-w-0 items-center gap-1">
        <p
          className="min-w-0 flex-1 truncate font-mono text-xs leading-relaxed text-muted-foreground"
          title={latest ?? undefined}
        >
          {latest ?? "Nothing reported yet."}
        </p>
        {onExpand === undefined ? null : (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="loom-grid-control h-6 w-6 shrink-0"
            aria-label={`Show the full ${label} log`}
            onClick={onExpand}
          >
            <Maximize2 aria-hidden="true" className="h-3.5 w-3.5" />
          </Button>
        )}
      </div>
      <span className="truncate text-xs text-muted-foreground" title={label}>
        {label}
      </span>
    </div>
  );
}
