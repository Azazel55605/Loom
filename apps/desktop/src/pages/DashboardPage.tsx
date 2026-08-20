import { DesktopAppShell } from "@/components/DesktopAppShell";
import { Dashboard } from "@loom/ui-kit/components/Dashboard";

export function DashboardPage() {
  return (
    <Dashboard renderShell={(content) => <DesktopAppShell>{content}</DesktopAppShell>} />
  );
}
