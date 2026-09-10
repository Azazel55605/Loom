import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";
import {
  HashRouter,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
} from "react-router-dom";

import { desktopBaseUrlProvider } from "@/adapters/desktopBaseUrlProvider";
import { createDesktopHttpTransport } from "@/adapters/desktopHttpTransport";
import { desktopServerProfileManager } from "@/adapters/desktopServerProfileManager";
import { desktopTokenStorage } from "@/adapters/desktopTokenStorage";
import {
  desktopInvalidCertificateWebSocketNote,
  desktopWebSocketTransport,
} from "@/adapters/desktopWebSocketTransport";
import {
  DesktopPermissionsIndexRedirect,
  DesktopPermissionsRoute,
} from "@/components/DesktopPermissionsRoute";
import { DesktopSettingsRoute } from "@/components/DesktopSettingsRoute";
import { ConnectorsPage } from "@/pages/ConnectorsPage";
import {
  DashboardDetailPage,
  DashboardsIndexPage,
} from "@/pages/DashboardsPage";
import { LoginPage } from "@/pages/LoginPage";
import { SetupPage } from "@/pages/SetupPage";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { BootScreen } from "@loom/ui-kit/components/BootScreen";
import { Button } from "@loom/ui-kit/components/ui/button";
import { AddServerFlow, type ServerConnection } from "@loom/ui-kit/components/ConnectToServer";
import {
  ServerSwitcherProvider,
  useServerSwitcher,
} from "@loom/ui-kit/components/ServerSwitcher";
import { Toaster } from "@loom/ui-kit/components/ui/sonner";
import { AuthProvider, useAuth } from "@loom/ui-kit/lib/auth-context";
import { useSetupStatus } from "@loom/ui-kit/lib/use-setup-status";

const AccountPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AccountPanel")).AccountPanel,
}));
const AppearancePanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AppearancePanel")).AppearancePanel,
}));
const UsersPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/UsersPanel")).UsersPanel,
}));
const GroupsPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/GroupsPanel")).GroupsPanel,
}));
const AuditLogPage = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AuditLogPage")).AuditLogPage,
}));
const DashboardsPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/DashboardsPanel")).DashboardsPanel,
}));
const DesktopUpdatesPanel = React.lazy(async () => ({
  default: (await import("@/components/DesktopUpdatesPanel")).DesktopUpdatesPanel,
}));

type ServerState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; connection: ServerConnection; profileId: string | null };

export default function App({ queryClient }: { queryClient: QueryClient }) {
  return (
    <HashRouter>
      <DesktopApplication queryClient={queryClient} />
    </HashRouter>
  );
}

function DesktopApplication({ queryClient }: { queryClient: QueryClient }) {
  const [server, setServer] = React.useState<ServerState>({ kind: "loading" });
  const navigate = useNavigate();

  const loadServer = React.useCallback(() => {
    setServer({ kind: "loading" });
    void Promise.all([
      desktopBaseUrlProvider.getConnection(),
      desktopServerProfileManager.getActiveProfileId(),
    ])
      .then(([connection, profileId]) =>
        setServer({ kind: "ready", connection, profileId }),
      )
      .catch((error: unknown) =>
        setServer({
          kind: "error",
          message:
            error instanceof Error ? error.message : "Desktop settings could not be read.",
        }),
      );
  }, []);

  React.useEffect(loadServer, [loadServer]);

  const activateCurrentProfile = React.useCallback(
    async (connection?: ServerConnection) => {
      if (connection !== undefined) {
        await desktopBaseUrlProvider.setConnection(connection);
      }
      const [nextConnection, profileId] = await Promise.all([
        desktopBaseUrlProvider.getConnection(),
        desktopServerProfileManager.getActiveProfileId(),
      ]);
      queryClient.clear();
      setServer({ kind: "ready", connection: nextConnection, profileId });
      navigate("/dashboards", { replace: true });
    },
    [navigate, queryClient],
  );

  const addFirstServer = async (connection: ServerConnection, label?: string) => {
    const profile = await desktopServerProfileManager.addProfile(
      label ?? connection.baseUrl,
      connection.baseUrl,
    );
    try {
      await desktopServerProfileManager.setActiveProfileId(profile.id);
      await activateCurrentProfile(connection);
    } catch (error) {
      await desktopServerProfileManager.removeProfile(profile.id).catch(() => undefined);
      throw error;
    }
  };

  const providerKey =
    server.kind === "ready"
      ? `${server.profileId ?? "none"}|${server.connection.baseUrl}`
      : server.kind;

  return (
    <ServerSwitcherProvider
      key={providerKey}
      manager={desktopServerProfileManager}
      supportsInvalidCertificates
      invalidCertificateNote={desktopInvalidCertificateWebSocketNote}
      getHttpTransport={createDesktopHttpTransport}
      onActiveProfileChanged={activateCurrentProfile}
    >
      <DesktopRuntime
        server={server}
        loadServer={loadServer}
        onAddFirstServer={addFirstServer}
      />
    </ServerSwitcherProvider>
  );
}

function DesktopRuntime({
  server,
  loadServer,
  onAddFirstServer,
}: {
  server: ServerState;
  loadServer: () => void;
  onAddFirstServer: (connection: ServerConnection, label?: string) => Promise<void>;
}) {
  const { openSwitcher } = useServerSwitcher();

  if (server.kind === "loading") return <BootScreen baseUrl="" />;

  if (server.kind === "error") {
    return (
      <main className="flex min-h-screen items-center justify-center p-6">
        <div className="flex w-full max-w-md flex-col gap-4">
          <Alert variant="destructive">
            <AlertCircle aria-hidden="true" />
            <AlertTitle>Desktop settings unavailable</AlertTitle>
            <AlertDescription>{server.message}</AlertDescription>
          </Alert>
          <Button variant="outline" onClick={loadServer}>
            Try again
          </Button>
        </div>
      </main>
    );
  }

  if (server.connection.baseUrl === "") {
    return (
      <AddServerFlow
        firstServer
        supportsInvalidCertificates
        invalidCertificateNote={desktopInvalidCertificateWebSocketNote}
        getHttpTransport={createDesktopHttpTransport}
        onConnected={onAddFirstServer}
      />
    );
  }

  const connectionKey = `${server.profileId}|${server.connection.baseUrl}|${server.connection.allowInvalidCertificates}`;

  return (
    <AuthProvider
      key={connectionKey}
      baseUrlProvider={desktopBaseUrlProvider}
      bootstrapBaseUrl={server.connection.baseUrl}
      onChangeServer={openSwitcher}
      httpTransport={createDesktopHttpTransport(
        server.connection.allowInvalidCertificates,
      )}
      tokenStorage={desktopTokenStorage}
      webSocketTransport={desktopWebSocketTransport}
    >
      <React.Suspense fallback={null}>
        <DesktopRoutes />
      </React.Suspense>
      <Toaster />
    </AuthProvider>
  );
}

function DesktopRoutes() {
  return (
    <RequireSetup>
      <Routes>
        <Route path="/setup" element={<SetupPage />} />
        <Route path="/login" element={<LoginPage />} />
        <Route path="/" element={<Navigate to="/dashboards" replace />} />
        <Route
          path="/dashboards"
          element={
            <RequireAuth>
              <DashboardsIndexPage />
            </RequireAuth>
          }
        />
        <Route
          path="/dashboards/:id"
          element={
            <RequireAuth>
              <DashboardDetailPage />
            </RequireAuth>
          }
        />
        <Route
          path="/connectors"
          element={
            <RequireAuth>
              <ConnectorsPage />
            </RequireAuth>
          }
        />
        <Route
          path="/settings"
          element={
            <RequireAuth>
              <DesktopSettingsRoute />
            </RequireAuth>
          }
        >
          <Route index element={<Navigate to="general" replace />} />
          <Route path="general" element={null} />
          <Route path="account" element={<AccountPanel />} />
          <Route path="appearance" element={<AppearancePanel />} />
          <Route path="updates" element={<DesktopUpdatesPanel />} />
          <Route path="audit-log" element={<AuditLogPage />} />
          <Route path="dashboards" element={<DashboardsPanel />} />
          <Route path="permissions" element={<DesktopPermissionsRoute />}>
            <Route index element={<DesktopPermissionsIndexRedirect />} />
            <Route path="users" element={<UsersPanel />} />
            <Route path="groups" element={<GroupsPanel />} />
            <Route path="*" element={<Navigate to="users" replace />} />
          </Route>
          <Route path="*" element={<Navigate to="general" replace />} />
        </Route>
        <Route path="*" element={<Navigate to="/dashboards" replace />} />
      </Routes>
    </RequireSetup>
  );
}

function RequireSetup({ children }: { children: React.ReactNode }) {
  const setup = useSetupStatus();
  const location = useLocation();
  if (setup.isPending) return null;
  if (setup.isError) return <>{children}</>;
  if (setup.data.setupComplete === false && location.pathname !== "/setup") {
    return <Navigate to="/setup" replace />;
  }
  return <>{children}</>;
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, isRestoring } = useAuth();
  const location = useLocation();
  if (isRestoring) return null;
  if (!isAuthenticated) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  return <>{children}</>;
}
