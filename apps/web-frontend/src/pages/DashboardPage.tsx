import { Dashboard } from "@loom/ui-kit/components/Dashboard";

import { WebAppShell } from "@/components/WebAppShell";

export function DashboardPage() {
  return (
    <Dashboard renderShell={(content) => <WebAppShell>{content}</WebAppShell>} />
  );
}
