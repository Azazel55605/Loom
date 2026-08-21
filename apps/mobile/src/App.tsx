import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { AlertCircle } from "lucide-react";
import {
  HashRouter,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";

import { mobileBaseUrlProvider } from "@/adapters/mobileBaseUrlProvider";
import { mobileTokenStorage } from "@/adapters/mobileTokenStorage";
import {
  MobilePermissionsIndexRedirect,
  MobilePermissionsRoute,
} from "@/components/MobilePermissionsRoute";
import { MobileSettingsRoute } from "@/components/MobileSettingsRoute";
import { ConnectorsPage } from "@/pages/ConnectorsPage";
import {
  DashboardDetailPage,
  DashboardsIndexPage,
} from "@/pages/DashboardsPage";
import { LoginPage } from "@/pages/LoginPage";
import { SetupPage } from "@/pages/SetupPage";
import { ConnectToServer } from "@loom/ui-kit/components/ConnectToServer";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Toaster } from "@loom/ui-kit/components/ui/sonner";
import { AuthProvider, useAuth } from "@loom/ui-kit/lib/auth-context";
import { useSetupStatus } from "@loom/ui-kit/lib/use-setup-status";

const AccountPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AccountPanel")).AccountPanel,
}));
const AppearancePanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AppearancePanel"))
    .AppearancePanel,
}));
const UsersPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/UsersPanel")).UsersPanel,
}));
const GroupsPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/GroupsPanel")).GroupsPanel,
}));

type ServerState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; baseUrl: string };

export default function App({ queryClient }: { queryClient: QueryClient }) {
  const [server, setServer] = React.useState<ServerState>({ kind: "loading" });

  const loadServer = React.useCallback(() => {
    setServer({ kind: "loading" });
    void mobileBaseUrlProvider
      .getBaseUrl()
      .then((baseUrl) => setServer({ kind: "ready", baseUrl }))
      .catch((error: unknown) =>
        setServer({
          kind: "error",
          message:
            error instanceof Error
              ? error.message
              : "Mobile settings could not be read.",
        }),
      );
  }, []);

  React.useEffect(loadServer, [loadServer]);

  if (server.kind === "loading") return null;

  if (server.kind === "error") {
    return (
      <main className="flex min-h-screen items-center justify-center p-6">
        <div className="flex w-full max-w-md flex-col gap-4">
          <Alert variant="destructive">
            <AlertCircle aria-hidden="true" />
            <AlertTitle>Mobile settings unavailable</AlertTitle>
            <AlertDescription>{server.message}</AlertDescription>
          </Alert>
          <Button variant="outline" onClick={loadServer}>
            Try again
          </Button>
        </div>
      </main>
    );
  }

  if (server.baseUrl === "") {
    return (
      <ConnectToServer
        onConnected={async ({ baseUrl }) => {
          await mobileBaseUrlProvider.setBaseUrl(baseUrl);
          setServer({ kind: "ready", baseUrl });
        }}
      />
    );
  }

  const changeServer = async (baseUrl: string) => {
    if (baseUrl === server.baseUrl) return;
    await mobileTokenStorage.clearTokens();
    await mobileBaseUrlProvider.setBaseUrl(baseUrl);
    queryClient.clear();
    setServer({ kind: "ready", baseUrl });
  };

  return (
    <HashRouter>
      <AuthProvider
        key={server.baseUrl}
        baseUrlProvider={mobileBaseUrlProvider}
        tokenStorage={mobileTokenStorage}
      >
        <React.Suspense fallback={null}>
          <MobileRoutes
            baseUrl={server.baseUrl}
            onServerChanged={changeServer}
          />
        </React.Suspense>
        <Toaster />
      </AuthProvider>
    </HashRouter>
  );
}

function MobileRoutes({
  baseUrl,
  onServerChanged,
}: {
  baseUrl: string;
  onServerChanged: (baseUrl: string) => Promise<void>;
}) {
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
              <MobileSettingsRoute
                baseUrl={baseUrl}
                onServerChanged={onServerChanged}
              />
            </RequireAuth>
          }
        >
          <Route index element={<Navigate to="general" replace />} />
          <Route path="general" element={null} />
          <Route path="account" element={<AccountPanel />} />
          <Route path="appearance" element={<AppearancePanel />} />
          <Route path="permissions" element={<MobilePermissionsRoute />}>
            <Route index element={<MobilePermissionsIndexRedirect />} />
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
