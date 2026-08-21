import { ConnectorsView } from "@loom/ui-kit/components/ConnectorsView";

import { WebAppShell } from "@/components/WebAppShell";

export function DashboardPage() {
  return (
    <ConnectorsView renderShell={(content) => <WebAppShell>{content}</WebAppShell>} />
  );
}
