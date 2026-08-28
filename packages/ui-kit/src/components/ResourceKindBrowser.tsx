import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { AlertCircle, Check, Loader2, X } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@loom/ui-kit/components/ui/table";
import {
  ActionParamsDialog,
  takesParameters,
} from "@loom/ui-kit/components/ActionParamsDialog";
import type {
  ColumnDescriptor,
  ConnectorAction,
  ResourceItem,
  ResourceKindDescriptor,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";
import { formatByteReading } from "@loom/ui-kit/widgets/types";

/** The params key a row action names its row with, per the API contract. */
const RESOURCE_ID_PARAM = "resourceId";

/** The column key a row names its sub-target with, per the API contract. */
const TARGET_ID_FIELD = "targetId";

/**
 * One connector's browsable resource kind, as a table.
 *
 * **Driven entirely by the descriptor.** The columns, their formatting, the
 * buttons beside each row and above the table all come from what the backend
 * published; there is no branch anywhere in here on which connector this is.
 * That is the whole point of the resource-browser contract: Docker's images,
 * volumes and updates are three instances of one shape, and a fourth connector
 * with a fifth kind renders here on the day it declares one, with no frontend
 * change.
 *
 * `resourceId` is added by this component rather than typed by the user. The
 * backend's convention carries a row action's target in `params.resourceId`,
 * and a user asked to retype the id of the row they just clicked would be being
 * asked to do the software's job — and would eventually mistype it.
 */
export function ResourceKindBrowser({
  instanceId,
  targetId,
  descriptor,
  disabled = false,
  disabledReason,
  className,
}: {
  instanceId: string;
  /** Sub-target to scope the listing to, when the caller has one. */
  targetId?: string | null;
  descriptor: ResourceKindDescriptor;
  /** Set for a caller without `connectors.control`, or an unreachable
   *  connector. Visibility only — the backend re-checks every request. */
  disabled?: boolean;
  disabledReason?: string | null;
  className?: string;
}) {
  const api = useApiClient();
  const [paramsFor, setParamsFor] = React.useState<PendingAction | null>(null);
  const [runningId, setRunningId] = React.useState<string | null>(null);

  const items = useQuery({
    queryKey: ["connector-resources", instanceId, descriptor.kind, targetId ?? null],
    queryFn: ({ signal }) =>
      api.getResourceItems(instanceId, descriptor.kind, targetId ?? null, signal),
  });

  const run = useMutation({
    mutationFn: ({ action, params, targetId: actionTarget }: PendingAction) =>
      api.executeConnectorAction(instanceId, action.id, params, actionTarget ?? null),
    onMutate: (pending) => {
      setRunningId(pendingKey(pending));
    },
    onSuccess: (result, pending) => {
      setParamsFor(null);
      // A 200 with `success: false` is the service reached and declining —
      // different from the request failing, and the toast says which.
      if (result.success) {
        toast.success(pending.action.label, { description: result.message });
      } else {
        toast.warning(`${pending.action.label} declined`, { description: result.message });
      }
    },
    onError: (error: unknown, pending) => {
      toast.error(`${pending.action.label} failed`, {
        description: describeConnectorError(error),
      });
    },
    onSettled: () => {
      setRunningId(null);
      // The table is the point: an update that was just applied should leave
      // the list it was listed in.
      void items.refetch();
    },
  });

  /** Runs an action, collecting parameters first when it declares any beyond
   *  the row id this component supplies. */
  function start(pending: PendingAction) {
    if (takesParameters(withoutSupplied(pending.action, pending.params))) {
      setParamsFor(pending);
    } else {
      run.mutate(pending);
    }
  }

  const columns = descriptor.columns;
  const hasRowActions = descriptor.rowActions.length > 0;

  return (
    <div className={cn("space-y-3", className)}>
      {descriptor.kindActions.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {descriptor.kindActions.map((action) => {
            const pending: PendingAction = { action, params: {}, targetId: targetId ?? null };
            return (
              <Button
                key={action.id}
                type="button"
                variant="outline"
                size="sm"
                disabled={disabled || run.isPending}
                title={disabledReason ?? action.description ?? undefined}
                onClick={() => start(pending)}
              >
                {runningId === pendingKey(pending) ? (
                  <Loader2 className="animate-spin" aria-hidden="true" />
                ) : null}
                {action.label}
                {takesParameters(action) ? <span aria-hidden="true">…</span> : null}
              </Button>
            );
          })}
        </div>
      ) : null}

      {items.isPending ? (
        <ResourceTableSkeleton columns={columns.length + (hasRowActions ? 1 : 0)} />
      ) : items.isError ? (
        // The backend's own words, not a generic failure: "this connector
        // instance has no resource kind named X" and "the registry is
        // rate-limiting this host" call for different next steps.
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertDescription>{describeConnectorError(items.error)}</AlertDescription>
        </Alert>
      ) : items.data.length === 0 ? (
        // A header row over nothing reads as broken. A sentence reads as an
        // answer, and for the `updates` table it is the answer people want.
        <p className="text-sm text-muted-foreground">Nothing here.</p>
      ) : (
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                {columns.map((column) => (
                  <TableHead key={column.key}>{column.label}</TableHead>
                ))}
                {hasRowActions ? (
                  <TableHead className="text-right">
                    <span className="sr-only">Actions</span>
                  </TableHead>
                ) : null}
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.data.map((item) => (
                <TableRow key={item.id}>
                  {columns.map((column) => (
                    <TableCell key={column.key} className="align-top">
                      <ResourceCell column={column} value={item.fields[column.key]} />
                    </TableCell>
                  ))}
                  {hasRowActions ? (
                    <TableCell className="text-right align-top">
                      <div className="flex justify-end gap-1">
                        {descriptor.rowActions.map((action) => {
                          const pending: PendingAction = {
                            action,
                            // The convention plus anything the row already
                            // answers, applied here so no call site has to
                            // remember it and no user retypes a visible cell.
                            params: paramsFromRow(action, item),
                            targetId: rowTarget(action, targetId, item),
                          };
                          return (
                            <Button
                              key={action.id}
                              type="button"
                              variant="ghost"
                              size="sm"
                              disabled={disabled || run.isPending}
                              title={disabledReason ?? action.description ?? undefined}
                              onClick={() => start(pending)}
                            >
                              {runningId === pendingKey(pending) ? (
                                <Loader2 className="animate-spin" aria-hidden="true" />
                              ) : null}
                              {action.label}
                              {takesParameters(
                                withoutSupplied(action, paramsFromRow(action, item)),
                              ) ? (
                                <span aria-hidden="true">…</span>
                              ) : null}
                            </Button>
                          );
                        })}
                      </div>
                    </TableCell>
                  ) : null}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      <ActionParamsDialog
        key={paramsFor === null ? "none" : pendingKey(paramsFor)}
        action={
          paramsFor === null ? null : withoutSupplied(paramsFor.action, paramsFor.params)
        }
        connectorName={descriptor.label}
        isPending={run.isPending}
        onOpenChange={(open) => {
          if (!open) setParamsFor(null);
        }}
        onSubmit={(params) => {
          if (paramsFor === null) return;
          // The collected fields, plus the row id the user was never asked for.
          run.mutate({ ...paramsFor, params: { ...paramsFor.params, ...params } });
        }}
      />
    </div>
  );
}

/** An action about to run, with everything it needs. */
type PendingAction = {
  action: ConnectorAction;
  params: Record<string, unknown>;
  targetId: string | null;
};

/** Identifies one in-flight invocation, so only its own button spins. */
function pendingKey(pending: PendingAction): string {
  return `${pending.action.id}:${String(pending.params[RESOURCE_ID_PARAM] ?? "")}`;
}

/**
 * Which sub-target a row action addresses.
 *
 * A kind browsed at the host level can list rows that each belong to a
 * *different* sub-target — Docker's "updates" table is one row per container —
 * so the row has to be able to say which. It does that by carrying a
 * `targetId` field, the same way it answers a parameter by carrying a
 * same-named one.
 *
 * The row's **id** is deliberately not used as a fallback. It is right for a
 * table whose rows happen to be targets and wrong for one whose rows are
 * anything else: "recently updated" is keyed by log entry, and guessing from
 * the id there sent an action at a container named after a log row. That was a
 * real failure, found by running it, and this is the fix — a row says what it
 * addresses, or it addresses whatever the browser is scoped to.
 */
function rowTarget(
  action: ConnectorAction,
  browsingTarget: string | null | undefined,
  item: ResourceItem,
): string | null {
  if (browsingTarget !== undefined && browsingTarget !== null) return browsingTarget;
  const declared = item.fields[TARGET_ID_FIELD];
  if (typeof declared === "string" && declared.length > 0) return declared;
  return action.targetId;
}

/**
 * The action's schema with everything the row already answers removed.
 *
 * Two things are supplied without asking: the row id, always, and any parameter
 * whose name matches a **column key on that row**. The second is what turns
 * "apply this update" from a dialog asking for an image reference that is
 * printed in the cell next to the button into a single click — and it is a
 * general rule, not a Docker one: a connector makes a parameter answerable by
 * naming a column after it.
 *
 * Stripping them is also what decides whether a dialog is needed at all. An
 * action whose every parameter is answered by its row runs on click; one with a
 * genuine question left still asks it.
 */
function withoutSupplied(action: ConnectorAction, supplied: Record<string, unknown>): ConnectorAction {
  const schema = action.paramsSchema;
  if (typeof schema !== "object" || schema === null) return action;

  const record = schema as Record<string, unknown>;
  const properties = record.properties;
  if (typeof properties !== "object" || properties === null) return action;

  const answered = (key: string) => key in supplied;
  const remaining = Object.fromEntries(
    Object.entries(properties as Record<string, unknown>).filter(([key]) => !answered(key)),
  );
  const required = Array.isArray(record.required)
    ? record.required.filter((key) => typeof key !== "string" || !answered(key))
    : record.required;

  return { ...action, paramsSchema: { ...record, properties: remaining, required } };
}

/**
 * The parameters a row can answer on the caller's behalf.
 *
 * The row id under the convention's key, plus any cell whose column key is
 * literally a parameter name in the action's schema.
 */
function paramsFromRow(action: ConnectorAction, item: ResourceItem): Record<string, unknown> {
  const supplied: Record<string, unknown> = { [RESOURCE_ID_PARAM]: item.id };

  const schema = action.paramsSchema;
  const properties =
    typeof schema === "object" && schema !== null
      ? (schema as Record<string, unknown>).properties
      : undefined;
  if (typeof properties === "object" && properties !== null) {
    for (const name of Object.keys(properties as Record<string, unknown>)) {
      const value = item.fields[name];
      if (value !== undefined && value !== null && value !== "") supplied[name] = value;
    }
  }
  return supplied;
}

/**
 * One cell, formatted by its column's declared type.
 *
 * The raw value is what crosses the wire, always — `bytes` is an exact byte
 * count and `timestamp` an ISO string — so all of the scaling and localizing
 * happens here, once, for every connector.
 */
function ResourceCell({ column, value }: { column: ColumnDescriptor; value: unknown }) {
  if (value === undefined || value === null) {
    // A row that genuinely has no value for a column is ordinary. An em dash
    // says "nothing here" where an empty cell says "something went wrong".
    return <span className="text-muted-foreground">—</span>;
  }

  switch (column.valueType) {
    case "bytes": {
      if (typeof value !== "number" || !Number.isFinite(value)) break;
      const { text, unit } = formatByteReading(value);
      return (
        <span className="tabular-nums" title={`${value} bytes`}>
          {text} {unit}
        </span>
      );
    }
    case "timestamp": {
      if (typeof value !== "string") break;
      const parsed = new Date(value);
      if (Number.isNaN(parsed.getTime())) break;
      // Localized for the reader, with the exact instant kept in the title —
      // "3 minutes ago" is friendlier and an ISO string is what you quote in a
      // bug report.
      return (
        <time dateTime={value} title={value} className="whitespace-nowrap">
          {parsed.toLocaleString()}
        </time>
      );
    }
    case "bool": {
      if (typeof value !== "boolean") break;
      return value ? (
        <>
          <Check className="h-4 w-4 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
          <span className="sr-only">Yes</span>
        </>
      ) : (
        <>
          <X className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
          <span className="sr-only">No</span>
        </>
      );
    }
    case "number": {
      if (typeof value !== "number" || !Number.isFinite(value)) break;
      return <span className="tabular-nums">{value.toLocaleString()}</span>;
    }
    case "text":
      break;
  }

  // Fallback for a value that does not match its declared type, and for
  // `text`. A connector that reports a string where it promised a number still
  // gets its value shown rather than a blank cell blamed on the user.
  return (
    <span className="break-all" title={typeof value === "string" ? value : undefined}>
      {typeof value === "string" ? value : JSON.stringify(value)}
    </span>
  );
}

function ResourceTableSkeleton({ columns }: { columns: number }) {
  return (
    <div className="space-y-2" aria-label="Loading resources">
      <Skeleton className="h-8 w-full" />
      {Array.from({ length: 3 }, (_, row) => (
        <div key={row} className="flex gap-2">
          {Array.from({ length: Math.max(1, columns) }, (_, cell) => (
            <Skeleton key={cell} className="h-6 flex-1" />
          ))}
        </div>
      ))}
    </div>
  );
}
