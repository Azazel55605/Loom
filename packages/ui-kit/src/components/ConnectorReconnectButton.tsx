import * as React from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@loom/ui-kit/components/ui/button";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import type { ConnectorStatusSnapshot } from "@loom/ui-kit/lib/api";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

export function ConnectorReconnectButton({
  instanceId,
  instanceName,
  onStatus,
  iconOnly = false,
  className,
}: {
  instanceId: string;
  instanceName: string;
  onStatus: (snapshot: ConnectorStatusSnapshot) => void;
  iconOnly?: boolean;
  className?: string;
}) {
  const api = useApiClient();
  const [pending, setPending] = React.useState(false);

  const retry = async () => {
    if (pending) return;
    setPending(true);
    try {
      const snapshot = await api.reconnectConnectorInstance(instanceId);
      onStatus(snapshot);
      const availability = connectorAvailability(snapshot);
      if (availability.tone === "down" || availability.tone === "degraded") {
        const description =
          availability.diagnosis ??
          availability.statusReason ??
          (snapshot.statusError === undefined
            ? "The connector is still unavailable."
            : describeConnectorError(snapshot.statusError));
        toast.warning(`${instanceName} is still ${availability.label.toLowerCase()}`, {
          description,
        });
      } else {
        toast.success(`${instanceName} reconnected`);
      }
    } catch (error) {
      toast.error(`Could not retry ${instanceName}`, {
        description: describeConnectorError(error),
      });
    } finally {
      setPending(false);
    }
  };

  return (
    <Button
      type="button"
      variant="ghost"
      size={iconOnly ? "icon" : "sm"}
      className={cn(iconOnly && "loom-grid-control h-7 w-7", className)}
      disabled={pending}
      aria-label={iconOnly ? `Retry ${instanceName} now` : undefined}
      title={iconOnly ? "Retry now" : undefined}
      onClick={(event) => {
        event.stopPropagation();
        void retry();
      }}
    >
      {pending ? (
        <Loader2 className="animate-spin" aria-hidden="true" />
      ) : (
        <RefreshCw aria-hidden="true" />
      )}
      {iconOnly ? null : "Retry now"}
    </Button>
  );
}
