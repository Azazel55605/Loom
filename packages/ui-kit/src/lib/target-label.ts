/**
 * How a placement's sub-target reads in a header.
 *
 * Pure and client-side by design: a tile knows its `targetId` and nothing else,
 * and a header that had to fetch the sub-target list to render its own title
 * would put a request — and a loading state, and a failure state — on the
 * critical path of a dashboard that already has everything it needs.
 *
 * The cost of that choice is that this file knows one connector's convention.
 * That is a deliberate, bounded trade: the prefix is documented in
 * `docs/API_CONTRACT.md` and guaranteed unambiguous by Docker's own naming
 * rules (a container name cannot contain a colon), and the fallback for
 * anything unrecognised is exactly what was rendered before — the raw id. A
 * second connector wanting decorated targets is the moment to move this onto
 * `SubTarget.kind` and pay for the lookup.
 */

/** Marks a Docker sub-target id as naming a Compose project. */
export const STACK_TARGET_PREFIX = "stack:";

/** What a target id should be shown as. */
export type TargetLabel = {
  /** The text to render. */
  text: string;
  /** Whether it is a stack, so a caller can pick an icon to match. */
  isStack: boolean;
};

/**
 * Renders one target id, or `null` for a host/aggregate placement.
 *
 * A bare id renders exactly as it did before this existed, so nothing about a
 * container placement changes.
 */
export function describeTarget(targetId: string | null | undefined): TargetLabel | null {
  if (targetId === null || targetId === undefined || targetId === "") return null;
  if (!targetId.startsWith(STACK_TARGET_PREFIX)) {
    return { text: targetId, isStack: false };
  }
  const project = targetId.slice(STACK_TARGET_PREFIX.length);
  // `stack:` naming no project is not a stack; showing "(stack)" for it would
  // be decorating an id that resolves to nothing.
  if (project === "") return { text: targetId, isStack: false };
  return { text: `${project} (stack)`, isStack: true };
}

/**
 * A connector's raw `SubTarget.kind` as a badge reads.
 *
 * Title-cased and otherwise untouched: the vocabulary belongs to the connector,
 * so a word this client has never seen still renders, capitalised, rather than
 * being dropped or mapped to a guess.
 */
export function describeTargetKind(kind: string): string | null {
  const trimmed = kind.trim();
  if (trimmed === "") return null;
  return trimmed.charAt(0).toLocaleUpperCase() + trimmed.slice(1);
}
