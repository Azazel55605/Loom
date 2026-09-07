import { cn } from "@loom/ui-kit/lib/utils";

function Skeleton({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("motion-content-reveal animate-pulse rounded-md bg-muted", className)}
      {...props}
    />
  );
}

export { Skeleton };
