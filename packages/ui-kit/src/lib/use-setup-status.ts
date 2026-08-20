import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import type { SetupStatus } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";

/** Query key for the setup status, shared so the wizard can invalidate it. */
export const SETUP_STATUS_QUERY_KEY = ["setup-status"] as const;

/**
 * Whether this instance still needs first-run setup.
 *
 * One query, shared by the routing gate and the wizard through React Query's
 * cache, so the question is asked once per load rather than once per consumer.
 *
 * Deliberately not retried: the answer decides which screen renders, so a slow
 * retry loop would leave the app blank. A failure is handled by the gate, which
 * carries on to the normal login flow rather than trapping the user behind a
 * question it could not get answered.
 */
export function useSetupStatus(): UseQueryResult<SetupStatus, Error> {
  const api = useApiClient();
  return useQuery({
    queryKey: SETUP_STATUS_QUERY_KEY,
    queryFn: ({ signal }) => api.getSetupStatus(signal),
    retry: false,
    // The answer flips at most once in a session, and the wizard invalidates it
    // explicitly when it does.
    staleTime: Infinity,
  });
}
