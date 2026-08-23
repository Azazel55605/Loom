import type { ConnectorError, ConnectorStatus, PendingOperation } from "@loom/ui-kit/lib/api";

/**
 * One reading of an instance, from whichever source the caller has: an
 * instance summary, a detail response, or a live socket frame. All four fields
 * arrive together on every one of them.
 */
export type ConnectorReading = {
  status: ConnectorStatus | null;
  statusError?: ConnectorError;
  pendingOperation?: PendingOperation | null;
  diagnosis?: string | null;
};

/** Badge variants the health states map onto, plus the overlay's own. */
export type AvailabilityTone = "healthy" | "degraded" | "down" | "unknown" | "pending";

/**
 * What a client should show for one instance, and whether it should let anyone
 * press anything.
 *
 * One function rather than the same three ternaries in the card, the modal and
 * the widget dispatcher — the rule that a pending operation outranks health is
 * exactly the kind of thing that gets implemented twice and then only fixed
 * once.
 */
export type ConnectorAvailability = {
  /** Ready to render: `"Performing: Restart"`, `"Down"`, `"No reading"`. */
  label: string;
  tone: AvailabilityTone;
  /** Whether an action is in flight right now. */
  isPending: boolean;
  /** Whether controls should be inert. */
  actionsDisabled: boolean;
  /**
   * Why controls are inert, phrased for a tooltip — `null` when they are not.
   * A disabled control with no explanation is indistinguishable from a broken
   * one, and this is the sentence that tells them apart.
   */
  unavailableReason: string | null;
  /** The network-level explanation for a Down instance, when there is one. */
  diagnosis: string | null;
};

const HEALTH_LABEL = {
  healthy: "Healthy",
  degraded: "Degraded",
  down: "Down",
  unknown: "Unknown",
} as const;

export function connectorAvailability(reading: ConnectorReading): ConnectorAvailability {
  const pending = reading.pendingOperation ?? null;

  // A pending operation wins over health, and this is the whole point of the
  // feature: a service mid-restart genuinely reports Down, and showing that is
  // accurate and useless. Actions stay enabled — the service is expected back,
  // and greying the controls out would make a routine restart look like an
  // outage from the other direction.
  if (pending !== null) {
    return {
      label: `Performing: ${pending.actionLabel}`,
      tone: "pending",
      isPending: true,
      actionsDisabled: false,
      unavailableReason: null,
      diagnosis: null,
    };
  }

  const health = reading.status?.health ?? null;

  // No reading at all: the poll itself failed, so Loom does not know what the
  // service is doing and certainly cannot ask it to do anything.
  if (health === null) {
    return {
      label: "No reading",
      tone: "unknown",
      isPending: false,
      actionsDisabled: true,
      unavailableReason: "Unavailable — connector is unreachable",
      diagnosis: reading.diagnosis ?? null,
    };
  }

  return {
    label: HEALTH_LABEL[health],
    tone: health,
    isPending: false,
    // Down only. A degraded service is still answering, and disabling the
    // restart button on the one connector someone is trying to fix would be
    // precisely backwards.
    actionsDisabled: health === "down",
    unavailableReason:
      health === "down" ? "Unavailable — connector is unreachable" : null,
    diagnosis: health === "down" ? (reading.diagnosis ?? null) : null,
  };
}
