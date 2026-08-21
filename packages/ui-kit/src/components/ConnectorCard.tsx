import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Loader2, Pencil, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import {
  SchemaForm,
  validateSchemaValues,
  type JsonSchema,
} from "@loom/ui-kit/components/SchemaForm";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorAction,
  type ConnectorInstanceSummary,
  type HealthState,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";

/**
 * One connector instance: what it is, how it is doing, and what can be done to
 * it.
 *
 * A composition of `Card`, `Badge`, and `Button` rather than a new primitive —
 * nothing here needs behaviour that Radix would own. The health colour comes
 * from the `Badge` variants added for it, so the status palette lives in the
 * component library and in the `--status-*` tokens, not at this call site.
 *
 * ## Two requests, one card
 *
 * The list endpoint carries name, type, status, and `displayFields`; it does
 * **not** carry `actions`, which are per-instance and can vary with
 * configuration and remote state. So the card renders everything it was handed
 * immediately and fetches its own detail for the rest. While that is in flight
 * only the action row is a `Skeleton` — skeletoning the whole card would make
 * already-present data appear to reload, which reads as a bug.
 *
 * ## The action buttons are not the widget system
 *
 * One plain `Button` per action, and a small generated form for the ones taking
 * parameters. The real widget-primitive rendering — `defaultLayout`,
 * `dataPoints`, gauges and charts bound to a grid — is a deliberate follow-up.
 * This is the generic fallback that keeps a connector operable until then.
 */

/** Human-facing label per health state. Separate from the badge variant so the
 *  wording can change without touching the palette, and vice versa. */
const HEALTH_LABEL: Record<HealthState, string> = {
  healthy: "Healthy",
  degraded: "Degraded",
  down: "Down",
  unknown: "Unknown",
};

/**
 * Formats the status timestamp for display.
 *
 * Locale-formatted rather than raw RFC 3339: the timestamp exists to tell a
 * person how fresh the reading is, and `2026-08-19T15:29:17.772526566Z` does
 * that badly. The raw value stays in the `title` for anyone who wants it.
 */
function formatChecked(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Whether an action's schema declares any parameters at all. */
function takesParameters(action: ConnectorAction): boolean {
  const schema = action.paramsSchema;
  if (typeof schema !== "object" || schema === null) return false;
  const properties = (schema as JsonSchema).properties;
  return properties !== undefined && Object.keys(properties).length > 0;
}

export function ConnectorCard({
  instance,
  onEdit,
  onDelete,
}: {
  /** The list-summary object. Rendered immediately; detail is fetched here. */
  instance: ConnectorInstanceSummary;
  /** Supplied only when the viewer may manage instances; the control is not
   *  rendered otherwise. */
  onEdit?: (instance: ConnectorInstanceSummary) => void;
  onDelete?: (instance: ConnectorInstanceSummary) => void;
}) {
  const api = useApiClient();
  const { user } = useAuth();
  const { id, name, connectorType, metadata, status, statusError, displayFields } = instance;

  // Visibility only. **Not a security boundary**: the backend checks
  // `connectors.control` on every action request, scoped to this instance id,
  // and a user who edits this away in a console still gets a 403. It is also
  // deliberately unscoped here — see the note in lib/permissions.ts for why a
  // menu answers a different question than an authorization check does.
  const canControl = hasPermission(user?.permissions ?? [], PERMISSION_KEYS.connectorsControl);

  const detail = useQuery({
    queryKey: ["connector-instance", id],
    queryFn: ({ signal }) => api.getConnectorInstance(id, signal),
    // A 403 or a vanished instance is not worth hammering; a transient network
    // failure is worth one retry.
    retry: (failureCount, error) =>
      !(error instanceof ApiError && (error.isForbidden || error.status === 404)) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 1,
  });

  // Which action is in flight, so only that button shows a spinner. The
  // mutation itself is shared: one per card, keyed by the action it is running.
  const [pendingActionId, setPendingActionId] = React.useState<string | null>(null);
  const [paramsFor, setParamsFor] = React.useState<ConnectorAction | null>(null);

  const mutation = useMutation({
    mutationFn: ({ action, params }: { action: ConnectorAction; params?: unknown }) =>
      api.executeConnectorAction(id, action.id, params),
    onMutate: ({ action }) => {
      setPendingActionId(action.id);
    },
    onSuccess: (result, { action }) => {
      setParamsFor(null);
      // A 200 with `success: false` means the service was reached and declined.
      // That is a different thing from the request failing, and the toast says
      // so rather than reporting a flat "error".
      if (result.success) {
        toast.success(`${name}: ${action.label}`, { description: result.message });
      } else {
        toast.warning(`${name}: ${action.label} declined`, { description: result.message });
      }
    },
    onError: (error: unknown, { action }) => {
      toast.error(`${name}: ${action.label} failed`, {
        description: describeConnectorError(error),
      });
    },
    onSettled: () => {
      setPendingActionId(null);
      // Detail carries the action list, which can shift with state, so it is
      // still refetched. Status itself arrives from the backend poller's
      // WebSocket update rather than starting a second ad-hoc list request.
      void detail.refetch();
    },
  });

  function runAction(action: ConnectorAction) {
    if (takesParameters(action)) {
      setParamsFor(action);
      return;
    }
    mutation.mutate({ action });
  }

  const actions = detail.data?.actions ?? [];
  const showActionRow = canControl && (detail.isPending || actions.length > 0);
  const hasManageControls = onEdit !== undefined || onDelete !== undefined;

  return (
    <Card className="surface-elevated">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <CardTitle>{name}</CardTitle>
            <CardDescription>
              {/* The connector *type*, not the instance id — the instance id is
                  a UUID and means nothing to a reader. The icon is an
                  identifier rather than a URL and there is no icon set wired up
                  yet, so showing the id keeps it visible and honest instead of
                  rendering a broken image. */}
              {connectorType}
              {metadata.icon !== null && ` · ${metadata.icon}`} · v{metadata.version}
            </CardDescription>
          </div>

          <div className="flex items-center gap-1">
            {status !== null ? (
              <Badge variant={status.health}>{HEALTH_LABEL[status.health]}</Badge>
            ) : (
              <Badge variant="unknown">No reading</Badge>
            )}

            {hasManageControls && (
              <>
                {onEdit !== undefined && (
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => onEdit(instance)}
                    title={`Edit ${name}`}
                  >
                    <Pencil aria-hidden="true" />
                    <span className="sr-only">Edit {name}</span>
                  </Button>
                )}
                {onDelete !== undefined && (
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => onDelete(instance)}
                    title={`Delete ${name}`}
                  >
                    <Trash2 aria-hidden="true" />
                    <span className="sr-only">Delete {name}</span>
                  </Button>
                )}
              </>
            )}
          </div>
        </div>
      </CardHeader>

      <CardContent className="space-y-3">
        {displayFields.length > 0 && (
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
            {displayFields.map((field) => (
              <React.Fragment key={field.label}>
                <dt className="text-muted-foreground">{field.label}</dt>
                <dd className="truncate font-medium" title={field.value}>
                  {field.value}
                </dd>
              </React.Fragment>
            ))}
          </dl>
        )}

        {status !== null ? (
          <p className="text-sm text-muted-foreground">
            Checked{" "}
            <time dateTime={status.lastChecked} title={status.lastChecked}>
              {formatChecked(status.lastChecked)}
            </time>
          </p>
        ) : (
          // A connector Loom could not get a reading from at all — distinct
          // from one that reported itself down, which is a successful check.
          // Also what an instance that failed to load at startup looks like.
          <Alert variant="destructive">
            <AlertTitle>Status unavailable</AlertTitle>
            <AlertDescription>
              {statusError !== undefined
                ? describeConnectorError(statusError)
                : "The connector did not report a status."}
            </AlertDescription>
          </Alert>
        )}
      </CardContent>

      {showActionRow && (
        <CardFooter className="flex flex-wrap gap-2">
          {detail.isPending ? (
            // Only this row. The card above it is already showing real data and
            // must not appear to reload.
            <>
              <Skeleton className="h-8 w-20" />
              <Skeleton className="h-8 w-16" />
            </>
          ) : (
            actions.map((action) => {
              const isPending = pendingActionId === action.id;
              return (
                <Button
                  key={action.id}
                  variant="outline"
                  size="sm"
                  title={action.description ?? undefined}
                  disabled={mutation.isPending}
                  onClick={() => runAction(action)}
                >
                  {isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
                  {action.label}
                  {takesParameters(action) && <span aria-hidden="true">…</span>}
                </Button>
              );
            })
          )}
        </CardFooter>
      )}

      <ActionParamsDialog
        key={paramsFor?.id ?? "none"}
        action={paramsFor}
        connectorName={name}
        isPending={mutation.isPending}
        onOpenChange={(open) => {
          if (!open) setParamsFor(null);
        }}
        onSubmit={(params) => {
          if (paramsFor !== null) mutation.mutate({ action: paramsFor, params });
        }}
      />
    </Card>
  );
}

/**
 * Collects an action's parameters before running it.
 *
 * Generated from the action's own `paramsSchema` through the same `SchemaForm`
 * the add-connector dialog uses, so an action gaining a parameter needs no
 * frontend change. Same subset limitation applies — string, number, and boolean
 * only; see `SchemaForm`.
 */
function ActionParamsDialog({
  action,
  connectorName,
  isPending,
  onOpenChange,
  onSubmit,
}: {
  action: ConnectorAction | null;
  connectorName: string;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (params: Record<string, unknown>) => void;
}) {
  const [values, setValues] = React.useState<Record<string, unknown>>({});
  const [errors, setErrors] = React.useState<Record<string, string>>({});

  if (action === null) return null;

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {action.label} — {connectorName}
          </DialogTitle>
          <DialogDescription>
            {action.description ?? "This action takes parameters."}
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-4"
          // See ConnectorInstanceDialog: native validation bubbles are a
          // browser-default control, and they would pre-empt the connector's
          // own `invalidParams` message.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            const found = validateSchemaValues(action.paramsSchema, values);
            setErrors(found);
            if (Object.keys(found).length === 0) onSubmit(values);
          }}
        >
          <SchemaForm
            schema={action.paramsSchema}
            value={values}
            onChange={setValues}
            errors={errors}
            disabled={isPending}
            idPrefix={`action-${action.id}`}
          />

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isPending}>
              {isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Run
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
