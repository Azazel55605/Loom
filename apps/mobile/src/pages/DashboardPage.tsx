import { MobileAppShell } from "@/components/MobileAppShell";
import { Dashboard } from "@loom/ui-kit/components/Dashboard";

export function DashboardPage() {
  return (
    <Dashboard renderShell={(content) => <MobileAppShell>{content}</MobileAppShell>} />
  );
}
