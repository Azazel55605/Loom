import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, Loader2, Search, XCircle } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type { DiscoveredResource } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";

type RowResult =
  | { state: "pending" }
  | { state: "success" }
  | { state: "failure"; message: string };

type BatchSummary = { successes: number; attempted: number };

/**
 * Discovers child resources through one configured connector and turns chosen
 * suggestions into ordinary connector instances. Creation is sequential so a
 * refusal remains attached to the resource that caused it.
 */
export function DiscoverResourcesDialog({
  open,
  instanceId,
  instanceName,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  instanceId: string;
  instanceName: string;
  onOpenChange: (open: boolean) => void;
  onCreated: () => Promise<void>;
}) {
  const api = useApiClient();
  const discovery = useQuery({
    queryKey: ["connector-discovery", instanceId],
    queryFn: ({ signal }) => api.discoverConnectorResources(instanceId, signal),
    enabled: open,
    retry: false,
  });

  const [selected, setSelected] = React.useState<Set<number>>(() => new Set());
  const [results, setResults] = React.useState<Record<number, RowResult>>({});
  const [summary, setSummary] = React.useState<BatchSummary | null>(null);
  const [isAdding, setIsAdding] = React.useState(false);

  React.useEffect(() => {
    if (discovery.data === undefined) return;
    setSelected(new Set(discovery.data.map((_resource, index) => index)));
    setResults({});
    setSummary(null);
  }, [discovery.data]);

  const resources = discovery.data ?? [];
  const selectable = resources
    .map((_resource, index) => index)
    .filter((index) => results[index]?.state !== "success");
  const allSelected = selectable.length > 0 && selectable.every((index) => selected.has(index));

  function selectAll() {
    setSelected(new Set(selectable));
  }

  function selectNone() {
    setSelected(new Set());
  }

  function setChecked(index: number, checked: boolean) {
    setSelected((current) => {
      const next = new Set(current);
      if (checked) next.add(index);
      else next.delete(index);
      return next;
    });
  }

  async function addSelected() {
    const indices = [...selected].sort((left, right) => left - right);
    if (indices.length === 0) return;

    setIsAdding(true);
    setSummary(null);
    let successes = 0;
    const failures: number[] = [];

    for (const index of indices) {
      const resource = resources[index];
      if (resource === undefined) continue;

      setResults((current) => ({ ...current, [index]: { state: "pending" } }));
      try {
        await api.createConnectorInstance({
          connectorType: resource.targetConnectorType,
          name: resource.suggestedName,
          config: resource.config,
        });
        successes += 1;
        setResults((current) => ({ ...current, [index]: { state: "success" } }));
      } catch (error: unknown) {
        failures.push(index);
        setResults((current) => ({
          ...current,
          [index]: { state: "failure", message: describeAdminFailure(error).message },
        }));
      }
    }

    setSelected(new Set(failures));
    setSummary({ successes, attempted: indices.length });
    setIsAdding(false);
    void onCreated();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!isAdding) onOpenChange(next);
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Discover through {instanceName}</DialogTitle>
          <DialogDescription>
            Choose which discovered resources Loom should add as connector instances.
          </DialogDescription>
        </DialogHeader>

        {discovery.isPending ? <DiscoverySkeleton /> : null}

        {discovery.isError ? (
          <Alert variant="destructive">
            <XCircle aria-hidden="true" />
            <AlertTitle>Discovery failed</AlertTitle>
            <AlertDescription>{describeAdminFailure(discovery.error).message}</AlertDescription>
          </Alert>
        ) : null}

        {discovery.isSuccess && resources.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            This connector did not find any resources to add.
          </p>
        ) : null}

        {discovery.isSuccess && resources.length > 0 ? (
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-sm text-muted-foreground">
                {selected.size} of {selectable.length} available selected
              </p>
              <div className="flex gap-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={isAdding || allSelected}
                  onClick={selectAll}
                >
                  Select all
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={isAdding || selected.size === 0}
                  onClick={selectNone}
                >
                  Select none
                </Button>
              </div>
            </div>

            <div className="flex flex-col gap-2">
              {resources.map((resource, index) => (
                <ResourceRow
                  key={`${resource.targetConnectorType}:${resource.suggestedName}:${index}`}
                  resource={resource}
                  index={index}
                  checked={selected.has(index)}
                  result={results[index]}
                  disabled={isAdding || results[index]?.state === "success"}
                  onCheckedChange={(checked) => setChecked(index, checked)}
                />
              ))}
            </div>
          </div>
        ) : null}

        {summary !== null ? (
          <Alert>
            <CheckCircle2 aria-hidden="true" />
            <AlertTitle>Discovery import complete</AlertTitle>
            <AlertDescription>
              {summary.successes} of {summary.attempted} added successfully.
            </AlertDescription>
          </Alert>
        ) : null}

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            disabled={isAdding}
            onClick={() => onOpenChange(false)}
          >
            {summary !== null && summary.successes === summary.attempted ? "Done" : "Close"}
          </Button>
          {resources.length > 0 ? (
            <Button type="button" disabled={isAdding || selected.size === 0} onClick={addSelected}>
              {isAdding ? (
                <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
              ) : (
                <Search data-icon="inline-start" aria-hidden="true" />
              )}
              Add selected
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ResourceRow({
  resource,
  index,
  checked,
  result,
  disabled,
  onCheckedChange,
}: {
  resource: DiscoveredResource;
  index: number;
  checked: boolean;
  result: RowResult | undefined;
  disabled: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  const checkboxId = `${React.useId()}-discovered-resource-${index}`;

  return (
    <div className="surface-panel flex items-start gap-3 rounded-md border p-3">
      <Checkbox
        id={checkboxId}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(next) => onCheckedChange(next === true)}
        aria-label={`Add ${resource.suggestedName}`}
      />
      <div className="min-w-0 flex-1">
        <Label htmlFor={checkboxId} className="font-medium">
          {resource.suggestedName}
        </Label>
        <p className="text-xs text-muted-foreground">{resource.targetConnectorType}</p>
        <ConfigSummary config={resource.config} />
        <RowResultView result={result} />
      </div>
    </div>
  );
}

function ConfigSummary({ config }: { config: unknown }) {
  if (typeof config !== "object" || config === null || Array.isArray(config)) {
    return <p className="mt-2 text-xs text-muted-foreground">No configuration fields.</p>;
  }

  const entries = Object.entries(config as Record<string, unknown>);
  if (entries.length === 0) {
    return <p className="mt-2 text-xs text-muted-foreground">No configuration fields.</p>;
  }

  const visible = entries.slice(0, 5);
  return (
    <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
      {visible.map(([key, value]) => (
        <React.Fragment key={key}>
          <dt>{key}:</dt>
          <dd className="truncate font-mono" title={compactValue(value)}>
            {compactValue(value)}
          </dd>
        </React.Fragment>
      ))}
      {entries.length > visible.length ? (
        <>
          <dt>More:</dt>
          <dd>{entries.length - visible.length} additional fields</dd>
        </>
      ) : null}
    </dl>
  );
}

function compactValue(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return `[${value.length} items]`;
  if (typeof value === "object") return `{${Object.keys(value).length} fields}`;
  return String(value);
}

function RowResultView({ result }: { result: RowResult | undefined }) {
  if (result === undefined) return null;
  if (result.state === "pending") {
    return (
      <p className="mt-2 flex items-center gap-1 text-xs text-muted-foreground" aria-live="polite">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
        Adding…
      </p>
    );
  }
  if (result.state === "success") {
    return (
      <p className="mt-2 flex items-center gap-1 text-xs font-medium text-primary" aria-live="polite">
        <CheckCircle2 className="size-3.5" aria-hidden="true" />
        Added
      </p>
    );
  }
  return (
    <p className="mt-2 flex items-start gap-1 text-xs text-destructive" aria-live="polite">
      <XCircle className="size-3.5 shrink-0" aria-hidden="true" />
      {result.message}
    </p>
  );
}

function DiscoverySkeleton() {
  return (
    <div className="flex flex-col gap-2" aria-label="Discovering resources">
      {[0, 1, 2].map((index) => (
        <div key={index} className="flex items-start gap-3 rounded-md border p-3">
          <Skeleton className="size-4" />
          <div className="flex flex-1 flex-col gap-2">
            <Skeleton className="h-4 w-48 max-w-full" />
            <Skeleton className="h-3 w-24" />
            <Skeleton className="h-3 w-full" />
          </div>
        </div>
      ))}
    </div>
  );
}
