import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Loader2, LogOut } from "lucide-react";
import { toast } from "sonner";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@loom/ui-kit/components/ui/alert-dialog";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type { UserSession } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";

export const userSessionsQueryKey = (userId: string) => ["user-sessions", userId] as const;

/** Shared active-session list for Account self-service and Users administration. */
export function SessionManager({
  userId,
  selfService,
  onSelfRevokedAll,
}: {
  userId: string;
  /** Adds “This device”, protects that row from individual revocation, and
   * uses the explicit log-out-everywhere warning. */
  selfService: boolean;
  /** Clears platform token storage after self-service revoke-all succeeds. */
  onSelfRevokedAll?: () => Promise<void>;
}) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [confirmingSession, setConfirmingSession] = React.useState<UserSession | null>(null);
  const [confirmingAll, setConfirmingAll] = React.useState(false);

  const sessions = useQuery({
    queryKey: userSessionsQueryKey(userId),
    queryFn: ({ signal }) => api.getUserSessions(userId, signal),
    retry: false,
  });

  const revokeOne = useMutation({
    mutationFn: (session: UserSession) => api.revokeUserSession(userId, session.id),
    onSuccess: async () => {
      setConfirmingSession(null);
      await queryClient.invalidateQueries({ queryKey: userSessionsQueryKey(userId) });
      toast.success("Session revoked.");
    },
    onError: (error: unknown) => {
      toast.error("Could not revoke the session", {
        description: describeAdminFailure(error).message,
      });
    },
  });

  const revokeAll = useMutation({
    mutationFn: () => api.revokeAllUserSessions(userId),
    onSuccess: async () => {
      setConfirmingAll(false);
      if (selfService && onSelfRevokedAll !== undefined) {
        await onSelfRevokedAll();
        return;
      }
      await queryClient.invalidateQueries({ queryKey: userSessionsQueryKey(userId) });
      toast.success("All sessions revoked.");
    },
    onError: (error: unknown) => {
      toast.error("Could not revoke all sessions", {
        description: describeAdminFailure(error).message,
      });
    },
  });

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col items-start justify-between gap-3 sm:flex-row sm:items-center">
        <p className="text-sm text-muted-foreground">
          {sessions.data?.length ?? 0} active {sessions.data?.length === 1 ? "session" : "sessions"}
        </p>
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={sessions.isPending || sessions.data?.length === 0}
          onClick={() => setConfirmingAll(true)}
        >
          <LogOut data-icon="inline-start" aria-hidden="true" />
          {selfService ? "Log out everywhere" : "Revoke all"}
        </Button>
      </div>

      {sessions.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-24 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      ) : sessions.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load active sessions</AlertTitle>
          <AlertDescription>{describeAdminFailure(sessions.error).message}</AlertDescription>
        </Alert>
      ) : sessions.data.length === 0 ? (
        <p className="rounded-md border p-4 text-sm text-muted-foreground">
          No active sessions.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {sessions.data.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              showCurrentDevice={selfService}
              onRevoke={setConfirmingSession}
            />
          ))}
        </ul>
      )}

      <AlertDialog
        open={confirmingSession !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmingSession(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke this session?</AlertDialogTitle>
            <AlertDialogDescription>
              That device will lose the ability to renew its access and will need to sign in
              again.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={revokeOne.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={revokeOne.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (confirmingSession !== null) revokeOne.mutate(confirmingSession);
              }}
            >
              {revokeOne.isPending ? (
                <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
              ) : null}
              Revoke
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={confirmingAll} onOpenChange={setConfirmingAll}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {selfService ? "Log out everywhere?" : "Revoke all sessions?"}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {selfService
                ? "This includes this device. You will be redirected to sign in again."
                : "Every active device for this user will need to sign in again."}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={revokeAll.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={revokeAll.isPending}
              onClick={(event) => {
                event.preventDefault();
                revokeAll.mutate();
              }}
            >
              {revokeAll.isPending ? (
                <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
              ) : null}
              {selfService ? "Log out everywhere" : "Revoke all"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function SessionRow({
  session,
  showCurrentDevice,
  onRevoke,
}: {
  session: UserSession;
  showCurrentDevice: boolean;
  onRevoke: (session: UserSession) => void;
}) {
  return (
    <li className="flex flex-col justify-between gap-3 rounded-md border p-4 sm:flex-row sm:items-center">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="break-words text-sm font-medium">
            {friendlyUserAgent(session.userAgent)}
          </p>
          {showCurrentDevice && session.isCurrent ? <Badge>This device</Badge> : null}
        </div>
        <dl className="mt-2 grid gap-x-4 gap-y-1 text-xs text-muted-foreground sm:grid-cols-[auto_1fr]">
          <dt>IP address</dt>
          <dd>{session.ipAddress ?? "Unavailable"}</dd>
          <dt>Started</dt>
          <dd><time dateTime={session.createdAt}>{formatDate(session.createdAt)}</time></dd>
          <dt>Expires</dt>
          <dd><time dateTime={session.expiresAt}>{formatDate(session.expiresAt)}</time></dd>
        </dl>
      </div>
      {showCurrentDevice && session.isCurrent ? null : (
        <Button type="button" variant="outline" size="sm" onClick={() => onRevoke(session)}>
          Revoke
        </Button>
      )}
    </li>
  );
}

function friendlyUserAgent(value: string | null): string {
  if (value === null) return "Unknown client";
  const os = value.includes("Android")
    ? "Android"
    : /iPhone|iPad/.test(value)
      ? "iOS"
      : value.includes("Windows")
        ? "Windows"
        : value.includes("Mac OS X")
          ? "macOS"
          : value.includes("Linux")
            ? "Linux"
            : null;
  const client = /; wv\)/.test(value)
    ? "Android WebView"
    : value.includes("Edg/")
      ? "Microsoft Edge"
      : /Firefox\//.test(value)
        ? "Firefox"
        : /Chrome\/|CriOS\//.test(value)
          ? "Chrome"
          : value.includes("Safari/")
            ? "Safari"
            : null;
  if (client !== null && os !== null) return `${client} on ${os}`;
  return client ?? os ?? value;
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
