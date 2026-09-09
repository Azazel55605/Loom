import * as React from "react";
import { onBackButtonPress } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { useNavigate } from "react-router-dom";

import { QuitConfirmDialog } from "@/components/QuitConfirmDialog";
import { useMobileKioskMode } from "@/components/mobileKioskMode";
import { BackHandlerStack } from "@loom/ui-kit/lib/back-handler-stack";

type RouterHistoryState = {
  idx?: unknown;
};

function hasRouterHistory(): boolean {
  const state = window.history.state as RouterHistoryState | null;
  return typeof state?.idx === "number" && state.idx > 0;
}

/** Android hardware/gesture back: overlay, then router, then explicit quit. */
export function MobileBackNavigation({ children }: { children: React.ReactNode }) {
  const navigate = useNavigate();
  const kiosk = useMobileKioskMode();
  const [quitOpen, setQuitOpen] = React.useState(false);

  const handleBackRef = React.useRef<() => void>(() => undefined);
  handleBackRef.current = () => {
    if (BackHandlerStack.handleBackPress()) return;

    // HashRouter stores its current stack index in history state. Kiosk mode is
    // deliberately chrome-free and is always treated as a presentation root,
    // even if ordinary app routes remain below it from before activation.
    if (!kiosk.enabled && hasRouterHistory()) {
      navigate(-1);
      return;
    }

    setQuitOpen(true);
  };

  React.useEffect(() => {
    if (!isTauri()) return undefined;

    let disposed = false;
    let listener: Awaited<ReturnType<typeof onBackButtonPress>> | undefined;
    void onBackButtonPress(() => handleBackRef.current()).then(
      (registered) => {
        if (disposed) {
          void registered.unregister();
        } else {
          listener = registered;
        }
      },
      () => {
        // The API only exists on Android. A failed registration leaves the
        // host platform's existing back behavior unchanged.
      },
    );

    return () => {
      disposed = true;
      if (listener !== undefined) void listener.unregister();
    };
  }, []);

  return (
    <>
      {children}
      <QuitConfirmDialog open={quitOpen} onOpenChange={setQuitOpen} />
    </>
  );
}
