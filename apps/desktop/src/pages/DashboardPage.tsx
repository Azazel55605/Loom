import { DesktopAppShell } from "@/components/DesktopAppShell";
import { ConnectorsView } from "@loom/ui-kit/components/ConnectorsView";

export function DashboardPage() {
  return (
    <ConnectorsView renderShell={(content) => <DesktopAppShell>{content}</DesktopAppShell>} />
  );
}
