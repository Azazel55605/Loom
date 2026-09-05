import * as React from "react";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";

import { ConnectorsPage } from "@/pages/ConnectorsPage";
import { DashboardDetailPage, DashboardsIndexPage } from "@/pages/DashboardsPage";
import { LoginPage } from "@/pages/LoginPage";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { useSetupStatus } from "@loom/ui-kit/lib/use-setup-status";

/**
 * Routes that are not on the way in, loaded on demand.
 *
 * The dashboards and the login screen stay in the main bundle — one of them is
 * the first thing every visit renders, so deferring them would only add a round
 * trip to the critical path. The rest are genuinely occasional:
 *
 * - **Setup** runs exactly once in an instance's life.
 * - **Settings** is administrative, and it is where the heavy dependencies are.
 *   The tables, dialogs, selects, checkboxes and switches under `/settings` pull
 *   in most of the Radix surface the app uses, and a user who never opens
 *   settings never needs any of it.
 *
 * Split at the route rather than per component: a route is a boundary the user
 * already understands as a navigation, so the load has somewhere natural to
 * happen. The four settings modules are separate `lazy` calls, but Rollup emits
 * them against shared chunks, so opening one tab does not re-download what the
 * next one needs.
 */
const SetupPage = React.lazy(async () => ({
  default: (await import("@/pages/SetupPage")).SetupPage,
}));
const SettingsLayout = React.lazy(async () => ({
  default: (await import("@/components/WebSettingsRoute")).WebSettingsRoute,
}));
const PermissionsLayout = React.lazy(async () => ({
  default: (await import("@/components/WebPermissionsRoute")).WebPermissionsRoute,
}));
const PermissionsIndexRedirect = React.lazy(async () => ({
  default: (await import("@/components/WebPermissionsRoute"))
    .WebPermissionsIndexRedirect,
}));
const AccountPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AccountPanel")).AccountPanel,
}));
const AppearancePanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/AppearancePanel")).AppearancePanel,
}));
const GeneralPanel = React.lazy(async () => ({
  default: (await import("@loom/ui-kit/pages/settings/GeneralPanel")).GeneralPanel,
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

/**
 * The route table.
 *
 * First-run setup and login are public; everything else is behind `RequireAuth`.
 * The guard is a route wrapper rather than a check inside each page so that
 * adding a page cannot accidentally add an unauthenticated one.
 *
 * Note that these are *client-side* guards, and client-side guards are UX, not
 * security. Nothing here keeps anyone away from data — the API is the only
 * enforcement point, per docs/adr/0003-auth-model-vpn-vs-external.md, and it
 * now checks a permission on every route (see docs/API_CONTRACT.md). The
 * settings sections are gated the same way: the tab is hidden, the data is
 * protected by the backend.
 */
export default function App() {
  return (
    <RequireSetup>
      {/* `null` while a route chunk is in flight, matching what `RequireSetup`
          and `RequireAuth` already do for an unresolved answer: render nothing
          rather than flash something that is about to be replaced. These chunks
          are served from the same origin as the page, so the gap is a fetch
          from cache in the ordinary case.

          One boundary around the whole table rather than one per route — a
          nested boundary would only matter if some routes wanted a different
          fallback, and they do not. */}
      <React.Suspense fallback={null}>
        <Routes>
          <Route path="/setup" element={<SetupPage />} />
          <Route path="/login" element={<LoginPage />} />
          <Route
            path="/"
            element={<Navigate to="/dashboards" replace />}
          />
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
                <SettingsLayout />
              </RequireAuth>
            }
          >
            {/* `/settings` alone is not a page — it is the shell around one.
                General is visible to every authenticated user, so it is always
                a valid landing point and needs no permission check. */}
            <Route index element={<Navigate to="/settings/general" replace />} />
            <Route path="general" element={<GeneralPanel />} />
            <Route path="account" element={<AccountPanel />} />
            <Route path="appearance" element={<AppearancePanel />} />
            <Route path="audit-log" element={<AuditLogPage />} />
            <Route path="dashboards" element={<DashboardsPanel />} />
            <Route path="permissions" element={<PermissionsLayout />}>
              {/* Which half to land on depends on which grant the user holds,
                  so the decision is a component rather than a fixed target. */}
              <Route index element={<PermissionsIndexRedirect />} />
              <Route path="users" element={<UsersPanel />} />
              <Route path="groups" element={<GroupsPanel />} />
              <Route path="*" element={<Navigate to="users" replace />} />
            </Route>
            {/* Kept inside the branch so an unknown settings path lands back in
                settings rather than on the dashboard. */}
            <Route path="*" element={<Navigate to="/settings/general" replace />} />
          </Route>

          {/* Unknown paths go to dashboards, which bounces to login when
              signed out. A dedicated 404 is worth having once there is more than
              one real page to be lost between. */}
          <Route path="*" element={<Navigate to="/dashboards" replace />} />
        </Routes>
      </React.Suspense>
    </RequireSetup>
  );
}

/**
 * Sends an unconfigured instance to the wizard, whatever was asked for.
 *
 * Setup outranks authentication: on an instance that has not been set up there
 * is no account to log in with, so bouncing to `/login` first would strand the
 * user on a form that cannot succeed. This wraps the whole route table rather
 * than sitting on individual routes so a new page is covered by default.
 */
function RequireSetup({ children }: { children: React.ReactNode }) {
  const setup = useSetupStatus();
  const location = useLocation();

  // Render nothing while the answer is outstanding. It decides which screen the
  // user belongs on, and flashing the wrong one first is worse than a blank
  // moment on a local request.
  if (setup.isPending) return null;

  // If the question could not be answered — a backend that does not serve this
  // route, or one that is down — carry on to the normal flow rather than
  // trapping the user in a wizard that cannot submit either. The login screen
  // then reports the real problem in terms they can act on.
  if (setup.isError) return <>{children}</>;

  if (setup.data.setupComplete === false && location.pathname !== "/setup") {
    return <Navigate to="/setup" replace />;
  }

  return <>{children}</>;
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, isRestoring } = useAuth();
  const location = useLocation();

  // Render nothing while a stored token is being validated. Redirecting on a
  // not-yet-known session would bounce a signed-in user to the login screen on
  // every reload, which reads as being randomly signed out.
  if (isRestoring) return null;

  if (!isAuthenticated) {
    // Remember where they were headed so signing in returns them there.
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }

  return <>{children}</>;
}
