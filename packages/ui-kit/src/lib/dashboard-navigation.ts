export type DashboardButtonNavigationState = {
  viaButtonNavigation: true;
  fromDashboardId: string;
};

/**
 * Router-independent state contract for dashboard-to-dashboard button tiles.
 *
 * Each host app owns its router; the shared package only defines and validates
 * the serializable marker that distinguishes a tile click from sidebar
 * navigation.
 */
export function dashboardButtonNavigationState(
  fromDashboardId: string,
): DashboardButtonNavigationState {
  return { viaButtonNavigation: true, fromDashboardId };
}

export function readDashboardButtonNavigationState(
  state: unknown,
): DashboardButtonNavigationState | null {
  if (typeof state !== "object" || state === null) return null;
  const candidate = state as Record<string, unknown>;
  return candidate.viaButtonNavigation === true &&
    typeof candidate.fromDashboardId === "string" &&
    candidate.fromDashboardId.length > 0
    ? {
        viaButtonNavigation: true,
        fromDashboardId: candidate.fromDashboardId,
      }
    : null;
}
