/**
 * Selects one addressable view from the nested ConnectorStatus.details shape.
 *
 * The backend uses an empty-string key for host/aggregate values and the exact
 * sub-target id otherwise. Keeping that sentinel here prevents each renderer
 * from inventing its own fallback or accidentally reading another target.
 */
export function statusDetailsForTarget(
  details: unknown,
  targetId: string | null,
): Record<string, unknown> {
  if (typeof details !== "object" || details === null || Array.isArray(details)) return {};
  const target = (details as Record<string, unknown>)[targetId ?? ""];
  return typeof target === "object" && target !== null && !Array.isArray(target)
    ? (target as Record<string, unknown>)
    : {};
}

/** Whether a descriptor belongs to the same view as a placement. */
export function matchesTarget(
  descriptor: { targetId: string | null },
  targetId: string | null,
): boolean {
  return descriptor.targetId === targetId;
}
