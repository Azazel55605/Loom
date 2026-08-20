import { Navigate, useLocation, useNavigate } from "react-router-dom";

import { LoginForm } from "@loom/ui-kit/components/LoginForm";
import { useAuth } from "@loom/ui-kit/lib/auth-context";

export function LoginPage() {
  const { isAuthenticated, isRestoring } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/";

  if (isRestoring) return null;
  if (isAuthenticated) return <Navigate to={from} replace />;

  return <LoginForm onSuccess={() => navigate(from, { replace: true })} />;
}
