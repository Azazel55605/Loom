import { CheckCircle2, XCircle } from "lucide-react";

import type { CapabilityStatus } from "@loom/ui-kit/lib/api";
import { cn } from "@loom/ui-kit/lib/utils";

/** One visual language for declarative and live connector capabilities. */
export function CapabilityStatusList({ capabilities }: { capabilities: CapabilityStatus[] }) {
  if (capabilities.length === 0) return null;

  return (
    <ul className="flex flex-col gap-2" aria-label="Connector capabilities">
      {capabilities.map((capability) => (
        <li key={capability.key} className="flex items-start gap-2 text-sm">
          {capability.available ? (
            <CheckCircle2
              className="mt-0.5 size-4 shrink-0 text-status-healthy"
              aria-hidden="true"
            />
          ) : (
            <XCircle className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          )}
          <span className="flex min-w-0 flex-col gap-0.5">
            <span
              className={cn(
                capability.available && "font-medium",
                !capability.available && "text-muted-foreground",
              )}
            >
              {capability.label}
            </span>
            {!capability.available && capability.note !== null ? (
              <span className="text-xs text-muted-foreground">{capability.note}</span>
            ) : null}
          </span>
        </li>
      ))}
    </ul>
  );
}
