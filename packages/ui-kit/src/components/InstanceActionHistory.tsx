import * as React from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";

import { AuditLogTable } from "@loom/ui-kit/components/AuditLogTable";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type { AuditLogFilters, ConnectorAction } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

export const AUDIT_LOG_PAGE_SIZE = 50;

/** Always-available History tab content for one connector instance. */
export function InstanceActionHistory({
  instanceId,
  actions,
}: {
  instanceId: string;
  actions: ConnectorAction[];
}) {
  const api = useApiClient();
  const [actionId, setActionId] = React.useState("all");
  const [outcome, setOutcome] = React.useState("all");
  const actionOptions = React.useMemo(
    () =>
      Array.from(new Map(actions.map((action) => [action.id, action.label])).entries()).sort(
        ([left], [right]) => left.localeCompare(right),
      ),
    [actions],
  );
  const filters = React.useMemo<AuditLogFilters>(
    () => ({
      actionId: actionId === "all" ? undefined : actionId,
      success: outcome === "all" ? undefined : outcome === "success",
      limit: AUDIT_LOG_PAGE_SIZE,
    }),
    [actionId, outcome],
  );

  const history = useInfiniteQuery({
    queryKey: ["connector-action-log", instanceId, filters],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      api.getInstanceActionLog(
        instanceId,
        { ...filters, before: pageParam },
        signal,
      ),
    getNextPageParam: (lastPage) =>
      lastPage.length < AUDIT_LOG_PAGE_SIZE
        ? undefined
        : lastPage[lastPage.length - 1]?.invokedAt,
  });
  const entries = React.useMemo(
    () => history.data?.pages.flat() ?? [],
    [history.data?.pages],
  );

  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <Select value={actionId} onValueChange={setActionId}>
          <SelectTrigger aria-label="Filter history by action">
            <SelectValue placeholder="All actions" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="all">All actions</SelectItem>
              {actionOptions.map(([id, label]) => (
                <SelectItem key={id} value={id}>
                  {label} ({id})
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
        <Select value={outcome} onValueChange={setOutcome}>
          <SelectTrigger aria-label="Filter history by outcome">
            <SelectValue placeholder="All outcomes" />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="all">All outcomes</SelectItem>
              <SelectItem value="success">Successful</SelectItem>
              <SelectItem value="failure">Failed</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      </div>

      {history.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      ) : history.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load action history</AlertTitle>
          <AlertDescription>{describeConnectorError(history.error)}</AlertDescription>
        </Alert>
      ) : entries.length === 0 ? (
        <p className="py-8 text-center text-sm text-muted-foreground">
          No actions match these filters.
        </p>
      ) : (
        <AuditLogTable entries={entries} />
      )}

      {history.hasNextPage ? (
        <Button
          type="button"
          variant="outline"
          className="self-center"
          disabled={history.isFetchingNextPage}
          onClick={() => void history.fetchNextPage()}
        >
          {history.isFetchingNextPage ? (
            <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
          ) : null}
          Load more
        </Button>
      ) : null}
    </div>
  );
}
