import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle, ChevronRight, File, Folder, Grid2X2, List, Loader2, Search } from "lucide-react";

import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type { BrowsableItem } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";
import { ImageDisplayWidget } from "@loom/ui-kit/widgets/ImageDisplay";
import type { WidgetExecute } from "@loom/ui-kit/widgets/types";

type PathPart = { id: string; label: string };
type ViewMode = "grid" | "list";

function metadataText(item: BrowsableItem): string {
  return Object.values(item.metadata)
    .filter((value): value is string | number | boolean =>
      typeof value === "string" || typeof value === "number" || typeof value === "boolean")
    .slice(0, 2)
    .map(String)
    .join(" · ");
}

/** A connector-neutral hierarchical content browser with shared grid/list data. */
export function BrowsableListGrid({
  instanceId,
  targetId,
  actionId,
  onExecute,
  disabled = false,
  className,
}: {
  instanceId: string;
  targetId?: string | null;
  actionId?: string | null;
  onExecute: WidgetExecute;
  disabled?: boolean;
  className?: string;
}) {
  const api = useApiClient();
  const [path, setPath] = React.useState<PathPart[]>([]);
  const [queryText, setQueryText] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [view, setView] = React.useState<ViewMode>("grid");

  React.useEffect(() => {
    const timeout = window.setTimeout(() => setDebouncedQuery(queryText.trim()), 250);
    return () => window.clearTimeout(timeout);
  }, [queryText]);

  const currentPath = path.at(-1)?.id ?? null;
  const result = useQuery({
    queryKey: ["connector-browsable-content", instanceId, targetId ?? null, currentPath, debouncedQuery],
    queryFn: ({ signal }) => debouncedQuery
      ? api.searchConnectorContent(instanceId, debouncedQuery, targetId, signal)
      : api.browseConnectorContent(instanceId, currentPath, targetId, signal),
  });

  const selection = useMutation({
    mutationFn: (item: BrowsableItem) => {
      if (!actionId) throw new Error("This browser has no selection action.");
      return onExecute(actionId, { itemId: item.id });
    },
  });

  const openItem = (item: BrowsableItem) => {
    if (item.kind === "container") {
      setQueryText("");
      setDebouncedQuery("");
      setPath((current) => [...current, { id: item.id, label: item.label }]);
    } else if (actionId) {
      selection.mutate(item);
    }
  };

  return (
    <section className={cn("flex min-h-0 min-w-0 flex-col gap-3", className)}>
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-[12rem] flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={queryText}
            onChange={(event) => setQueryText(event.target.value)}
            placeholder="Search content"
            aria-label="Search browsable content"
            className="pl-9"
          />
        </div>
        <SegmentedControl
          label="Content view"
          value={view}
          onChange={setView}
          options={[
            { value: "grid", label: "Grid", icon: <Grid2X2 /> },
            { value: "list", label: "List", icon: <List /> },
          ]}
        />
      </div>

      {!debouncedQuery ? (
        <nav aria-label="Content path" className="flex min-h-11 flex-wrap items-center gap-1 text-sm">
          <Button variant="ghost" size="sm" onClick={() => setPath([])}>Root</Button>
          {path.map((part, index) => (
            <React.Fragment key={part.id}>
              <ChevronRight className="size-4 text-muted-foreground" aria-hidden="true" />
              <Button variant="ghost" size="sm" onClick={() => setPath((current) => current.slice(0, index + 1))}>
                {part.label}
              </Button>
            </React.Fragment>
          ))}
        </nav>
      ) : null}

      {result.isLoading ? <BrowserSkeleton view={view} /> : null}
      {result.isError ? (
        <Alert variant="destructive">
          <AlertCircle className="size-4" />
          <AlertDescription>{describeConnectorError(result.error)}</AlertDescription>
        </Alert>
      ) : null}
      {result.data?.length === 0 ? (
        <div className="flex min-h-32 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
          Nothing here.
        </div>
      ) : null}

      {result.data && result.data.length > 0 ? (
        <div className={cn(view === "grid" ? "grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-3" : "flex flex-col gap-1")}>
          {result.data.map((item) => {
            const actionable = item.kind === "container" || Boolean(actionId);
            const pending = selection.isPending && selection.variables?.id === item.id;
            const meta = metadataText(item);
            const content = view === "grid" ? (
              <>
                {item.thumbnail ? (
                  <ImageDisplayWidget label={item.label} value={item.thumbnail} config={{ fit: "cover" }} />
                ) : (
                  <div className="flex aspect-square min-h-32 items-center justify-center rounded-lg bg-muted/55 text-muted-foreground">
                    {item.kind === "container" ? <Folder className="!size-10" /> : <File className="!size-10" />}
                  </div>
                )}
                {!item.thumbnail ? <span className="break-words font-medium">{item.label}</span> : null}
                {meta ? <span className="break-words text-xs text-muted-foreground">{meta}</span> : null}
              </>
            ) : (
              <>
                {item.kind === "container" ? <Folder className="size-5 shrink-0" /> : <File className="size-5 shrink-0" />}
                <span className="min-w-0 flex-1 break-words text-left font-medium">{item.label}</span>
                {meta ? <span className="hidden text-xs text-muted-foreground sm:inline">{meta}</span> : null}
                {item.kind === "container" ? <ChevronRight className="size-4 shrink-0" /> : null}
              </>
            );

            return actionable ? (
              <Button
                key={item.id}
                type="button"
                variant="outline"
                className={cn(
                  "relative h-auto min-w-0 whitespace-normal",
                  view === "grid" ? "flex-col items-stretch justify-start gap-2 p-[var(--card-padding)]" : "justify-start gap-3 px-3 py-2",
                )}
                disabled={pending || (item.kind === "leaf" && disabled)}
                onClick={() => openItem(item)}
              >
                {pending ? <Loader2 className="absolute right-2 top-2 size-4 animate-spin" /> : null}
                {content}
              </Button>
            ) : (
              <div key={item.id} className={cn("min-w-0 rounded-md border", view === "grid" ? "space-y-2 p-[var(--card-padding)]" : "flex min-h-11 items-center gap-3 px-3 py-2")}>
                {content}
              </div>
            );
          })}
        </div>
      ) : null}
    </section>
  );
}

function BrowserSkeleton({ view }: { view: ViewMode }) {
  return view === "grid" ? (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-3">
      {[0, 1, 2].map((item) => <Skeleton key={item} className="aspect-square min-h-32 rounded-lg" />)}
    </div>
  ) : (
    <div className="space-y-2">
      {[0, 1, 2].map((item) => <Skeleton key={item} className="h-11 w-full" />)}
    </div>
  );
}
