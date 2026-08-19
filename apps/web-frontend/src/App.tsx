import { Navigate, Route, Routes, useLocation } from "react-router-dom";

import { DashboardPage } from "@/pages/DashboardPage";
import { LoginPage } from "@/pages/LoginPage";
import { SetupPage } from "@/pages/SetupPage";
import { useAuth } from "@/lib/auth-context";
import { useSetupStatus } from "@/lib/use-setup-status";

/**
 * The route table.
 *
 * Three routes: first-run setup and login are public, everything else is behind
 * `RequireAuth`. The guard is a route wrapper rather than a check inside each
 * page so that adding a page cannot accidentally add an unauthenticated one.
 *
 * Note that these are *client-side* guards, and client-side guards are UX, not
 * security. Nothing here keeps anyone away from data — the API is the only
 * enforcement point, per docs/adr/0003-auth-model-vpn-vs-external.md. Today the
 * stub backend enforces nothing at all.
 */
export default function App() {
  return (
    <RequireSetup>
      <Routes>
        <Route path="/setup" element={<SetupPage />} />
        <Route path="/login" element={<LoginPage />} />
        <Route
          path="/"
          element={
            <RequireAuth>
              <DashboardPage />
            </RequireAuth>
          }
        />
        {/* Unknown paths go to the dashboard, which bounces to login when
            signed out. A dedicated 404 is worth having once there is more than
            one real page to be lost between. */}
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
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
