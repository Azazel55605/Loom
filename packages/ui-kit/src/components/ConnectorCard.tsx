import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
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
  type ConnectorAction,
  type ConnectorSummary,
  type HealthState,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/**
 * One connector: what it is, how it is doing, and what can be done to it.
 *
 * A composition of `Card`, `Badge`, and `Button` rather than a new primitive —
 * nothing here needs behaviour that Radix would own. The health colour comes
 * from the `Badge` variants added for it, so the status palette lives in the
 * component library and in the `--status-*` tokens, not at this call site.
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

export function ConnectorCard({
  connector,
  onActionComplete,
}: {
  connector: ConnectorSummary;
  /** Called after any action resolves, so the list can refetch its status. */
  onActionComplete?: () => void;
}) {
  const api = useApiClient();
  const { metadata, status, statusError, actions } = connector;

  // Which action is in flight, so only that button shows a spinner. The
  // mutation itself is shared: one per card, keyed by the action it is running.
  const [pendingActionId, setPendingActionId] = React.useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (action: ConnectorAction) => api.executeAction(metadata.id, action.id),
    onMutate: (action: ConnectorAction) => {
      setPendingActionId(action.id);
    },
    onSuccess: (result, action) => {
      // A 200 with `success: false` means the service was reached and declined.
      // That is a different thing from the request failing, and the toast says
      // so rather than reporting a flat "error".
      if (result.success) {
        toast.success(`${metadata.name}: ${action.label}`, {
          description: result.message,
        });
      } else {
        toast.warning(`${metadata.name}: ${action.label} declined`, {
          description: result.message,
        });
      }
    },
    onError: (error: unknown, action) => {
      toast.error(`${metadata.name}: ${action.label} failed`, {
        description: describeConnectorError(error),
      });
    },
    onSettled: () => {
      setPendingActionId(null);
      onActionComplete?.();
    },
  });

  return (
    <Card className="surface-elevated">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1">
            <CardTitle>{metadata.name}</CardTitle>
            <CardDescription>
              {/* The icon is an identifier, not a URL, and there is no icon set
                  wired up yet — showing the id keeps it visible and honest
                  instead of rendering a broken image. */}
              {metadata.id}
              {metadata.icon !== null && ` · ${metadata.icon}`} · v
              {metadata.version}
            </CardDescription>
          </div>

          {status !== null ? (
            <Badge variant={status.health}>{HEALTH_LABEL[status.health]}</Badge>
          ) : (
            <Badge variant="unknown">No reading</Badge>
          )}
        </div>
      </CardHeader>

      <CardContent className="space-y-3">
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

      {actions.length > 0 && (
        <CardFooter className="flex flex-wrap gap-2">
          {actions.map((action) => {
            const isPending = pendingActionId === action.id;
            return (
              <Button
                key={action.id}
                variant="outline"
                size="sm"
                title={action.description ?? undefined}
                disabled={mutation.isPending}
                onClick={() => mutation.mutate(action)}
              >
                {isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
                {action.label}
              </Button>
            );
          })}
        </CardFooter>
      )}
    </Card>
  );
}
