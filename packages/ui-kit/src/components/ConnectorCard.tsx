import * as React from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Loader2, Pencil, ScanSearch, Trash2 } from "lucide-react";
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
  ActionParamsDialog,
  takesParameters,
} from "@loom/ui-kit/components/ActionParamsDialog";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { ActionButtonSkeleton } from "@loom/ui-kit/widgets/ActionButton";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorAction,
  type ConnectorInstanceSummary,
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

// The health-state wording used to live here. It now lives in
// `lib/connector-availability.ts` alongside the rule that a pending operation
// outranks it, because the two were always one decision and keeping them apart
// meant this card could disagree with a dashboard tile about the same instance.

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

export function ConnectorCard({
  instance,
  onDiscover,
  onEdit,
  onDelete,
}: {
  /** The list-summary object. Rendered immediately; detail is fetched here. */
  instance: ConnectorInstanceSummary;
  /** Supplied only when the viewer may manage instances; the control is not
   *  rendered otherwise. */
  onDiscover?: (instance: ConnectorInstanceSummary) => void;
  onEdit?: (instance: ConnectorInstanceSummary) => void;
  onDelete?: (instance: ConnectorInstanceSummary) => void;
}) {
  const api = useApiClient();
  const { user } = useAuth();
  const { id, name, connectorType, metadata, iconOverride, status, statusError, displayFields } =
    instance;
  // The same rule the dashboard tiles use: a pending operation outranks health,
  // and an unreachable connector cannot be asked to do anything.
  const availability = connectorAvailability(instance);

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

  // The management card represents the connector instance itself. Sub-target
  // actions belong on target-scoped dashboard placements, where their target
  // identity is visible and explicitly threaded through execution.
  const actions = detail.data?.actions.filter((action) => action.targetId === null) ?? [];
  const showActionRow = canControl && (detail.isPending || actions.length > 0);
  const canDiscover =
    onDiscover !== undefined &&
    detail.data?.discoverableType !== null &&
    detail.data?.discoverableType !== undefined;
  const hasManageControls = canDiscover || onEdit !== undefined || onDelete !== undefined;

  return (
    <Card className="surface-elevated">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 items-start gap-3">
            {/* Larger here than on a dashboard card: this is the connector
                *administration* list, where a person is scanning a flat list of
                every instance and the icon is the fastest way to find one. */}
            <ConnectorIcon
              typeIcon={metadata.icon}
              iconOverride={iconOverride}
              size={28}
              className="mt-0.5"
            />
            <div className="min-w-0 space-y-1">
              <CardTitle>{name}</CardTitle>
              <CardDescription>
                {/* The connector *type*, not the instance id — the instance id
                    is a UUID and means nothing to a reader. The icon reference
                    itself is no longer printed here: it is drawn to the left,
                    which is what it was always for. */}
                {connectorType} · v{metadata.version}
              </CardDescription>
            </div>
          </div>

          <div className="flex items-center gap-1">
            <Badge variant={availability.tone}>{availability.label}</Badge>

            {hasManageControls && (
              <>
                {canDiscover ? (
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => onDiscover(instance)}
                    title={`Discover resources through ${name}`}
                  >
                    <ScanSearch aria-hidden="true" />
                    <span className="sr-only">Discover resources through {name}</span>
                  </Button>
                ) : null}
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
          <div className="flex flex-col gap-1">
            {availability.statusReason !== null ? (
              <p className="text-sm leading-relaxed text-muted-foreground">
                {availability.statusReason}
              </p>
            ) : null}
            <p className="text-sm text-muted-foreground">
              Checked{" "}
              <time dateTime={status.lastChecked} title={status.lastChecked}>
                {formatChecked(status.lastChecked)}
              </time>
            </p>
          </div>
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
              <ActionButtonSkeleton className="w-20" />
              <ActionButtonSkeleton className="w-16" />
            </>
          ) : (
            actions.map((action) => {
              const isPending = pendingActionId === action.id;
              return (
                <Button
                  key={action.id}
                  variant="outline"
                  size="sm"
                  title={availability.unavailableReason ?? action.description ?? undefined}
                  // An unreachable connector cannot carry out an action, so the
                  // button says so before the click rather than after it.
                  disabled={mutation.isPending || availability.actionsDisabled}
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
