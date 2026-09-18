import { Loader2 } from "lucide-react";

/**
 * Shown while a kiosk display holds a session it cannot currently verify.
 *
 * Deliberately not `KioskRecoveryScreen`: nothing has rejected this session, so
 * asking for credentials would be wrong twice over — the display is unattended,
 * and the session it already holds is still valid. It waits instead, for as
 * long as the outage lasts.
 */
export function KioskReconnectingScreen({ serverBaseUrl }: { serverBaseUrl: string }) {
  return (
    <main
      className="mobile-kiosk-recovery flex min-h-screen flex-col items-center justify-center gap-3 px-6 text-center"
      role="status"
    >
      <Loader2 className="h-8 w-8 animate-spin text-primary" aria-hidden="true" />
      <h1 className="text-lg font-semibold">Reconnecting…</h1>
      <p className="text-sm text-muted-foreground">
        This display is waiting for the server. It stays signed in and returns on its own.
      </p>
      {serverBaseUrl ? (
        <p className="break-all text-xs text-muted-foreground opacity-80">{serverBaseUrl}</p>
      ) : null}
    </main>
  );
}
