import * as React from "react";
import { TriangleAlert, X } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import type { NetworkAdvisoryUpdate } from "@loom/ui-kit/lib/connector-socket";

export function NetworkAdvisoryBanner() {
  const socket = useConnectorStatusSocket();
  const [advisory, setAdvisory] = React.useState<NetworkAdvisoryUpdate | null>(null);
  const [dismissed, setDismissed] = React.useState(false);
  const wasActive = React.useRef(false);

  React.useEffect(
    () =>
      socket.subscribeNetworkAdvisory((next) => {
        if (next.active && !wasActive.current) setDismissed(false);
        if (!next.active) setDismissed(false);
        wasActive.current = next.active;
        setAdvisory(next);
      }),
    [socket],
  );

  if (advisory?.active !== true || dismissed) return null;

  return (
    <Alert className="mx-auto mt-3 w-[calc(100%-2rem)] max-w-7xl border-amber-500/40 bg-amber-500/10 pr-14">
      <TriangleAlert aria-hidden="true" />
      <AlertTitle>Multiple services appear unreachable</AlertTitle>
      <AlertDescription>
        This may indicate a network issue affecting the Loom server itself, rather than a problem
        with your individual services. {advisory.affectedHostCount} distinct hosts are affected.
      </AlertDescription>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="absolute right-2 top-1/2 -translate-y-1/2"
        aria-label="Dismiss network advisory"
        onClick={() => setDismissed(true)}
      >
        <X aria-hidden="true" />
      </Button>
    </Alert>
  );
}
