import * as React from "react";

import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { configNumber, type DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

export function LogStreamSkeleton({ className, expanded = false }: { className?: string; expanded?: boolean }) {
  return <div className={cn("flex flex-col gap-2", expanded ? "min-h-[20rem]" : "min-h-[8rem]", className)}><Skeleton className="h-3 w-20" /><div className="surface-panel flex flex-1 flex-col gap-2 rounded-md border p-3"><Skeleton className="h-3 w-full" /><Skeleton className="h-3 w-5/6" /><Skeleton className="h-3 w-3/4" /></div></div>;
}

/** Below this many pixels from the bottom, the pane is treated as "following"
 *  and re-pins itself on the next update. Above it, the reader has scrolled up
 *  deliberately and is left alone. */
const FOLLOW_THRESHOLD_PX = 24;

/**
 * A scrolling pane of text lines.
 *
 * The value is a single `String` data point, newline-separated — not an array.
 * That is the `ConnectorStatus.details` contract: a data point declares one
 * value type, and inventing a second wire shape for the same declared type is
 * how a renderer and a connector quietly disagree.
 *
 * **Its height is capped and it scrolls inside itself.** A log is the one
 * reading with no natural maximum size, so the pane is bounded and the overflow
 * goes to its own scrollbar. Nothing about how long the log is may reach the
 * layout around it.
 *
 * **Auto-scroll follows, it does not force.** Pinning to the bottom on every
 * update would yank a reader out of the line they were reading, so the pane
 * only re-pins when it was already at the bottom when the update arrived. The
 * scroll is applied without smooth behaviour on purpose — a stream that
 * animates every frame is unreadable, and this is one of the places where
 * motion communicates nothing.
 */
export function LogStreamWidget({ label, value, config, className, expanded = false }: DisplayWidgetProps & { expanded?: boolean }) {
  const scroller = React.useRef<HTMLDivElement>(null);
  const wasAtBottom = React.useRef(true);

  const text = typeof value === "string" ? value : "";
  const lines = text.length === 0 ? [] : text.split("\n");
  const maxLines = configNumber(config, "maxLines", 0);
  const visible = maxLines > 0 ? lines.slice(-maxLines) : lines;

  React.useEffect(() => {
    const element = scroller.current;
    if (element === null || !wasAtBottom.current) return;
    element.scrollTop = element.scrollHeight;
  }, [text]);

  return (
    <div className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className="whitespace-normal text-xs text-muted-foreground [overflow-wrap:anywhere]">
        {label}
      </span>
      <div
        ref={scroller}
        onScroll={(event) => {
          const element = event.currentTarget;
          wasAtBottom.current =
            element.scrollHeight - element.scrollTop - element.clientHeight <= FOLLOW_THRESHOLD_PX;
        }}
        role="log"
        aria-label={label}
        // `role="log"` implies `aria-live="polite"`, which would have a screen
        // reader read out every line of a pane that updates on every poll.
        // Muted deliberately: the region stays navigable and readable on
        // demand, it just does not interrupt.
        aria-live="off"
        tabIndex={0}
        className={cn(
          "surface-panel min-h-0 flex-1 overflow-y-auto rounded-md border p-2 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          // The cap is the whole point: without it the pane is as tall as the
          // log is long, and a widget that grows without limit drags its tile
          // — and the grid rows either side of it — along with it.
          expanded ? "max-h-[28rem] min-h-[20rem]" : "max-h-[14rem]",
        )}
      >
        {visible.length === 0 ? (
          <p className="text-xs text-muted-foreground">Nothing reported yet.</p>
        ) : (
          <pre className="whitespace-pre-wrap break-words font-mono text-[0.7rem] leading-relaxed">
            {visible.join("\n")}
          </pre>
        )}
      </div>
    </div>
  );
}
