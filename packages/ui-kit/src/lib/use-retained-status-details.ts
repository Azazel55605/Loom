import * as React from "react";

/** Keeps the last defined value for each point so a sparse later socket frame
 * updates in place instead of making a previously rendered widget look new. */
export function useRetainedStatusDetails(
  instanceId: string,
  details: Record<string, unknown>,
): Record<string, unknown> {
  const retained = React.useRef<{ instanceId: string; values: Record<string, unknown> }>({
    instanceId,
    values: {},
  });
  if (retained.current.instanceId !== instanceId) {
    retained.current = { instanceId, values: {} };
  }
  for (const [key, value] of Object.entries(details)) {
    if (value !== undefined) retained.current.values[key] = value;
  }
  return { ...retained.current.values, ...details };
}
