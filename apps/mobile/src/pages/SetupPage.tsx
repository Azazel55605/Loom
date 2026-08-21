import { Navigate, useNavigate } from "react-router-dom";

import { SetupForm } from "@loom/ui-kit/components/SetupForm";
import { useSetupStatus } from "@loom/ui-kit/lib/use-setup-status";

export function SetupPage() {
  const navigate = useNavigate();
  const setup = useSetupStatus();
  if (setup.data?.setupComplete === true) return <Navigate to="/login" replace />;
  return <SetupForm onComplete={() => navigate("/login", { replace: true })} />;
}
