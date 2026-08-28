import { Info } from "lucide-react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@loom/ui-kit/components/ui/popover";
import type { ConnectorAvailability } from "@loom/ui-kit/lib/connector-availability";

/** Health badge plus an optional, click/tap-accessible explanation. */
export function ConnectorStatusBadge({
  availability,
}: {
  availability: ConnectorAvailability;
}) {
  const badge = <Badge variant={availability.tone}>{availability.label}</Badge>;

  if (availability.statusReason === null) return badge;

  return (
    <div className="flex items-center gap-1">
      {badge}
      <Popover>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="loom-grid-control"
            aria-label={`Why is this connector ${availability.label.toLowerCase()}?`}
          >
            <Info aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="end"
          aria-label={`${availability.label} status details`}
        >
          <div className="flex flex-col gap-1">
            <p className="text-sm font-semibold">{availability.label} status</p>
            <p className="text-sm leading-relaxed text-muted-foreground">
              {availability.statusReason}
            </p>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
