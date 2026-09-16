import * as React from "react";

import type { MediaDuration, PlaybackState } from "@loom/ui-kit/lib/api";

function seconds(duration: MediaDuration | null | undefined): number | null {
  if (duration === null || duration === undefined) return null;
  return Math.max(0, duration[0] + duration[1] / 1_000_000_000);
}

/** Advances the displayed position locally between authoritative server reads. */
export function usePlaybackPosition(state: PlaybackState | undefined): number {
  const serverPosition = seconds(state?.position) ?? 0;
  const duration = seconds(state?.currentItem?.duration);
  const [position, setPosition] = React.useState(serverPosition);

  React.useEffect(() => setPosition(serverPosition), [serverPosition]);
  React.useEffect(() => {
    if (state?.status !== "playing" || duration === null) return;
    const timer = window.setInterval(
      () => setPosition((current) => Math.min(duration, current + 1)),
      1_000,
    );
    return () => window.clearInterval(timer);
  }, [duration, state?.status]);

  return position;
}
