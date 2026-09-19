/**
 * Cache keys shared by the dashboard view and anything that warms it.
 *
 * Kept out of the components themselves so a tile can prefetch the dashboard it
 * navigates to without importing the view that renders it, which would close an
 * import cycle between the two.
 */

/** Every dashboard the signed-in user can open, as the sidebar lists them. */
export const dashboardsQueryKey = ["dashboards"] as const;

/** One dashboard's structure and placements. */
export const dashboardQueryKey = (dashboardId: string) =>
  ["dashboard", dashboardId] as const;

/**
 * How long a warmed dashboard counts as fresh.
 *
 * Long enough that arriving on the dashboard reuses what the prefetch fetched
 * instead of immediately fetching it again, short enough that a layout edited
 * elsewhere is not served stale for long. The view refetches on its own terms
 * after that.
 */
export const DASHBOARD_PREFETCH_STALE_MS = 30_000;
