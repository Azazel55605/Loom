import * as React from "react";
import { ImageOff } from "lucide-react";

import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { configString, type DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

type ImageLoadState = {
  src: string;
  status: "loaded" | "failed";
};

type ImageDisplayProps = DisplayWidgetProps & {
  expanded?: boolean;
};

/** Accepts only the two source forms promised by `DataPointValueType::Image`. */
function imageSource(value: unknown): string | null {
  if (typeof value !== "string" || value.trim() === "") return null;
  const source = value.trim();
  if (source.startsWith("data:")) return source;

  try {
    const parsed = new URL(source);
    return parsed.protocol === "http:" || parsed.protocol === "https:" ? source : null;
  } catch {
    return null;
  }
}

function frameClass(expanded: boolean): string {
  return expanded ? "h-[min(24rem,52vh)] min-h-[14rem]" : "aspect-square min-h-[8rem]";
}

/** Loading geometry shared by initial status loading and source transitions. */
export function ImageDisplaySkeleton({
  className,
  expanded = false,
}: {
  className?: string;
  expanded?: boolean;
}) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-2", className)}>
      <Skeleton className={cn("w-full rounded-lg", frameClass(expanded))} />
      <Skeleton className="h-3 w-24" />
    </div>
  );
}

/** A safe image surface with explicit loading and failure states. */
export function ImageDisplayWidget({
  label,
  value,
  config,
  className,
  expanded = false,
}: ImageDisplayProps) {
  const source = imageSource(value);
  const fit = configString(config, "fit", "contain") === "cover" ? "cover" : "contain";
  const [loadState, setLoadState] = React.useState<ImageLoadState | null>(null);
  const status = source === null
    ? "failed"
    : loadState?.src === source
      ? loadState.status
      : "loading";

  return (
    <figure className={cn("flex min-w-0 flex-col gap-2", className)}>
      <div
        className={cn(
          "relative w-full overflow-hidden rounded-lg border border-border/70 bg-muted/45",
          frameClass(expanded),
        )}
      >
        {status === "loading" ? <Skeleton className="absolute inset-0 size-full rounded-none" /> : null}
        {status === "failed" ? (
          <div
            className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-muted px-4 text-center text-muted-foreground"
            role="img"
            aria-label={`${label}: image unavailable`}
          >
            <ImageOff className="size-8" aria-hidden="true" />
            <span className="text-sm">Image unavailable</span>
          </div>
        ) : null}
        {source !== null && status !== "failed" ? (
          <img
            key={source}
            src={source}
            alt={label}
            className={cn(
              "absolute inset-0 size-full transition-opacity duration-200",
              fit === "cover" ? "object-cover" : "object-contain",
              status === "loaded" ? "opacity-100" : "opacity-0",
            )}
            onLoad={() => setLoadState({ src: source, status: "loaded" })}
            onError={() => setLoadState({ src: source, status: "failed" })}
          />
        ) : null}
      </div>
      <figcaption className="min-w-0 break-words text-xs text-muted-foreground">
        {label}
      </figcaption>
    </figure>
  );
}
