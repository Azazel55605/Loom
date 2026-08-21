import { MobileAppShell } from "@/components/MobileAppShell";
import { ConnectorsView } from "@loom/ui-kit/components/ConnectorsView";

export function ConnectorsPage() {
  return (
    <ConnectorsView renderShell={(content) => <MobileAppShell>{content}</MobileAppShell>} />
  );
}
