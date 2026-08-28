import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowUpCircle, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import type { ResourceKindDescriptor } from "@loom/ui-kit/lib/api";

/** The resource kind a connector publishes waiting updates as. */
export const UPDATES_KIND = "updates";

/**
 * How often the tile re-counts waiting updates.
 *
 * A minute, and deliberately nothing like the status cadence. Status arrives
 * pushed over the WebSocket because it changes second to second; update
 * availability is established by a scheduler that talks to a registry every few
 * *hours*, so polling it at status speed would be asking the same question
 * sixty times to hear the same answer. This is a count of rows in a table Loom
 * already holds — it costs no registry traffic — but there is still nothing to
 * learn from asking more often.
 */
export const UPDATE_COUNT_POLL_MS = 60_000;

/**
 * The waiting-updates badge and its inline "apply everything" button.
 *
 * Only rendered for a host-level placement whose connector actually declares an
 * `updates` kind, and only when that kind currently has rows: a tile for a
 * connector that has nothing to update looks exactly as it did before this
 * existed. The point of putting it on the tile rather than only in the detail
 * modal is that "three of my containers are behind" is a thing to notice in
 * passing, not something to go looking for.
 *
 * Generic in the same way the browser is: the kind, its label, and the
 * whole-kind action all come from the descriptor. Nothing here knows Docker.
 */
export function UpdatesSummary({
  instanceId,
  descriptor,
  disabled = false,
  disabledReason,
}: {
  instanceId: string;
  descriptor: ResourceKindDescriptor;
  disabled?: boolean;
  disabledReason?: string | null;
}) {
  const api = useApiClient();

  const items = useQuery({
    queryKey: ["connector-resources", instanceId, descriptor.kind, null],
    queryFn: ({ signal }) => api.getResourceItems(instanceId, descriptor.kind, null, signal),
    refetchInterval: UPDATE_COUNT_POLL_MS,
  });

  // The kind's own whole-collection action, whatever it is called. Taking the
  // first one rather than matching a name keeps this from being "the Docker
  // update button" wearing a generic coat.
  const applyAll = descriptor.kindActions[0];

  const run = useMutation({
    mutationFn: () => api.executeConnectorAction(instanceId, applyAll!.id, {}, null),
    onSuccess: (result) => {
      if (result.success) {
        toast.success(applyAll!.label, { description: result.message });
      } else {
        toast.warning(`${applyAll!.label} declined`, { description: result.message });
      }
    },
    onError: (error: unknown) => {
      toast.error(`${applyAll!.label} failed`, { description: describeConnectorError(error) });
    },
    onSettled: () => {
      // Whatever happened, the table is the record of what is still waiting.
      void items.refetch();
    },
  });

  const count = items.data?.length ?? 0;
  if (count === 0) return null;

  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <Badge variant="secondary" className="gap-1">
        <ArrowUpCircle className="h-3.5 w-3.5" aria-hidden="true" />
        {count === 1 ? "1 update available" : `${count} updates available`}
      </Badge>
      {applyAll === undefined ? null : (
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="loom-grid-control h-7"
          disabled={disabled || run.isPending}
          title={disabledReason ?? applyAll.description ?? undefined}
          onClick={() => run.mutate()}
        >
          {run.isPending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}
          {applyAll.label}
        </Button>
      )}
    </div>
  );
}
