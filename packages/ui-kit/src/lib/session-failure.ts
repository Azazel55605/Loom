import { ApiError, SessionExpiredError } from "@loom/ui-kit/lib/api";

/**
 * Backoff for reconnecting to a backend that is not answering.
 *
 * The same shape `ConnectorStatusSocket` reconnects with, and the same shape the
 * backend's own poller retries a connector with: start at a second, double,
 * settle at half a minute. One shape, so a device waiting out an outage behaves
 * the same whichever layer noticed the outage first.
 */
export const INITIAL_RETRY_DELAY_MS = 1_000;
export const MAX_RETRY_DELAY_MS = 30_000;

export function nextRetryDelayMs(current: number): number {
  return Math.min(current * 2, MAX_RETRY_DELAY_MS);
}

/**
 * Whether a failure means the *session itself* is over.
 *
 * True only for an explicit rejection by the server: a 401, or the
 * `SessionExpiredError` the client raises once a refresh token has been
 * rejected. Everything else — a DNS failure, a refused connection, a timeout, a
 * 502 from a proxy in front of a backend that is still booting — is the network
 * failing to deliver an answer, and says nothing whatsoever about whether the
 * stored refresh token is still valid.
 *
 * The distinction matters because the two call for opposite responses. A
 * rejected session must be cleared and re-entered; an undelivered request must
 * be retried and must never cost the user their session, least of all on an
 * unattended wall-mounted display whose wifi drops for ten seconds.
 */
export function isSessionRejection(error: unknown): boolean {
  if (error instanceof SessionExpiredError) return true;
  return error instanceof ApiError && error.isUnauthorized;
}
