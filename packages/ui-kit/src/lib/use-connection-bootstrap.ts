import * as React from "react";

import { INITIAL_RETRY_DELAY_MS, nextRetryDelayMs } from "@loom/ui-kit/lib/session-failure";

export type ConnectionBootstrapState =
  | { phase: "idle"; error: null }
  | { phase: "checking"; error: null }
  | { phase: "connected"; error: null }
  | { phase: "error"; error: string };

/**
 * Checks one configured backend before the auth runtime starts.
 *
 * A failed check is a statement about the *network*, never about the stored
 * session, so it never touches tokens and never becomes a sign-in prompt. It
 * retries on its own with the shared backoff instead of waiting for someone to
 * press a button: an unattended display has nobody to press it, and a check
 * that fails during a ten-second outage would otherwise park the app on an
 * error screen until the next reboot.
 */
export function useConnectionBootstrap(
  baseUrl: string | null,
  healthCheck: (signal?: AbortSignal) => Promise<boolean>,
  timeoutMs = 8_000,
) {
  const [attempt, setAttempt] = React.useState(0);
  const [state, setState] = React.useState<ConnectionBootstrapState>({ phase: "idle", error: null });
  const retryDelayMs = React.useRef(INITIAL_RETRY_DELAY_MS);

  React.useEffect(() => {
    if (!baseUrl) {
      retryDelayMs.current = INITIAL_RETRY_DELAY_MS;
      setState({ phase: "idle", error: null });
      return;
    }

    const controller = new AbortController();
    let timedOut = false;
    let retryTimer: ReturnType<typeof globalThis.setTimeout> | null = null;

    const scheduleRetry = () => {
      const delay = retryDelayMs.current;
      retryDelayMs.current = nextRetryDelayMs(delay);
      retryTimer = globalThis.setTimeout(() => {
        retryTimer = null;
        setAttempt((current) => current + 1);
      }, delay);
    };

    const fail = (error: string) => {
      setState({ phase: "error", error });
      scheduleRetry();
    };

    const timeout = globalThis.setTimeout(() => {
      timedOut = true;
      controller.abort();
      fail(`The server did not respond within ${Math.ceil(timeoutMs / 1_000)} seconds.`);
    }, timeoutMs);
    setState({ phase: "checking", error: null });

    void healthCheck(controller.signal)
      .then((healthy) => {
        if (controller.signal.aborted) return;
        if (healthy) {
          retryDelayMs.current = INITIAL_RETRY_DELAY_MS;
          setState({ phase: "connected", error: null });
          return;
        }
        fail("The server did not report a healthy response.");
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted && !timedOut) return;
        if (timedOut) return;
        fail(error instanceof Error ? error.message : "The server could not be reached.");
      })
      .finally(() => globalThis.clearTimeout(timeout));

    return () => {
      globalThis.clearTimeout(timeout);
      if (retryTimer !== null) globalThis.clearTimeout(retryTimer);
      controller.abort();
    };
  }, [attempt, baseUrl, healthCheck, timeoutMs]);

  const retry = React.useCallback(() => {
    retryDelayMs.current = INITIAL_RETRY_DELAY_MS;
    setAttempt((current) => current + 1);
  }, []);

  /** An automatic retry is pending, so the error screen is not a dead end. */
  const retrying = state.phase === "error";
  return { ...state, retrying, retry };
}
