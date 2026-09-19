/**
 * The cache key for the signed-in user's own account.
 *
 * Shared rather than declared per consumer, because the account is now read in
 * places that have nothing to do with the settings panel that owns it — the
 * dashboards index reads `defaultDashboardId` to decide where to land, and the
 * dashboard header writes it. Three components spelling `["account"]`
 * themselves works right up until one of them writes and the others do not
 * notice.
 */
export const accountQueryKey = ["account"] as const;
