import * as React from "react";

export type ConnectionBootstrapState =
  | { phase: "idle"; error: null }
  | { phase: "checking"; error: null }
  | { phase: "connected"; error: null }
  | { phase: "error"; error: string };

/** Checks one configured backend before the auth runtime starts. */
export function useConnectionBootstrap(
  baseUrl: string | null,
  healthCheck: (signal?: AbortSignal) => Promise<boolean>,
  timeoutMs = 8_000,
) {
  const [attempt, setAttempt] = React.useState(0);
  const [state, setState] = React.useState<ConnectionBootstrapState>({ phase: "idle", error: null });

  React.useEffect(() => {
    if (!baseUrl) {
      setState({ phase: "idle", error: null });
      return;
    }

    const controller = new AbortController();
    let timedOut = false;
    const timeout = globalThis.setTimeout(() => {
      timedOut = true;
      controller.abort();
      setState({
        phase: "error",
        error: `The server did not respond within ${Math.ceil(timeoutMs / 1_000)} seconds.`,
      });
    }, timeoutMs);
    setState({ phase: "checking", error: null });

    void healthCheck(controller.signal)
      .then((healthy) => {
        if (controller.signal.aborted) return;
        setState(
          healthy
            ? { phase: "connected", error: null }
            : { phase: "error", error: "The server did not report a healthy response." },
        );
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted && !timedOut) return;
        setState({
          phase: "error",
          error: timedOut
            ? `The server did not respond within ${Math.ceil(timeoutMs / 1_000)} seconds.`
            : error instanceof Error
              ? error.message
              : "The server could not be reached.",
        });
      })
      .finally(() => globalThis.clearTimeout(timeout));

    return () => {
      globalThis.clearTimeout(timeout);
      controller.abort();
    };
  }, [attempt, baseUrl, healthCheck, timeoutMs]);

  const retry = React.useCallback(() => setAttempt((current) => current + 1), []);
  return { ...state, retry };
}
