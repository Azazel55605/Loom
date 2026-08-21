import { ConnectorsView } from "@loom/ui-kit/components/ConnectorsView";

import { WebAppShell } from "@/components/WebAppShell";

export function ConnectorsPage() {
  return (
    <ConnectorsView renderShell={(content) => <WebAppShell>{content}</WebAppShell>} />
  );
}
