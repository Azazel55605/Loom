import { Navigate, Route, Routes, useLocation } from "react-router-dom";

import { DashboardPage } from "@/pages/DashboardPage";
import { LoginPage } from "@/pages/LoginPage";
import { useAuth } from "@/lib/auth-context";

/**
 * The route table.
 *
 * Two routes for now: a public login screen and everything else behind
 * `RequireAuth`. The guard is a route wrapper rather than a check inside each
 * page so that adding a page cannot accidentally add an unauthenticated one.
 *
 * Note that this is a *client-side* guard, and client-side guards are UX, not
 * security. Nothing here keeps anyone away from data — the API is the only
 * enforcement point, per docs/adr/0003-auth-model-vpn-vs-external.md. Today the
 * stub backend enforces nothing at all.
 */
export default function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route
        path="/"
        element={
          <RequireAuth>
            <DashboardPage />
          </RequireAuth>
        }
      />
      {/* Unknown paths go to the dashboard, which bounces to login when signed
          out. A dedicated 404 is worth having once there is more than one real
          page to be lost between. */}
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { token, isRestoring } = useAuth();
  const location = useLocation();

  // Render nothing while a stored token is being validated. Redirecting on a
  // not-yet-known session would bounce a signed-in user to the login screen on
  // every reload, which reads as being randomly signed out.
  if (isRestoring) return null;

  if (token === null) {
    // Remember where they were headed so signing in returns them there.
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }

  return <>{children}</>;
}
