import type { CapabilityRequirement, CapabilityStatus } from "@loom/ui-kit/lib/api";

const PLACEHOLDER = /\{\{([A-Za-z0-9_.-]+)\}\}/g;

export type CapabilitySummary = {
  capabilities: CapabilityStatus[];
  summarySentence: string;
};

function humanizeToggleKey(key: string): string {
  const spaced = key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[-_]+/g, " ")
    .trim();
  return spaced === "" ? key : spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

function joinLabels(labels: string[]): string {
  if (labels.length <= 1) return labels[0] ?? "";
  if (labels.length === 2) return `${labels[0]} and ${labels[1]}`;
  return `${labels.slice(0, -1).join(", ")}, and ${labels[labels.length - 1]}`;
}

function valueAtPath(values: Record<string, unknown>, path: string): unknown {
  let current: unknown = values;
  for (const segment of path.split(".")) {
    if (typeof current !== "object" || current === null) return undefined;
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

function templateValue(value: unknown, fieldName: string): string {
  if (value === undefined || value === null || value === "") return `<${fieldName}>`;
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }

  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/** Applies config-field and environment-toggle placeholders without mutation. */
export function renderSetupGuideTemplate(
  template: string,
  values: Record<string, unknown>,
): string {
  return template.replace(PLACEHOLDER, (_placeholder, fieldName: string) =>
    templateValue(valueAtPath(values, fieldName), fieldName),
  );
}

/**
 * Resolves the v1 AND-only setup-guide capability model without network I/O.
 *
 * Sanity cases: all required toggles true yields only available capabilities;
 * all false yields only unavailable capabilities and their missing-toggle
 * notes; a mixed map produces both clauses in the summary sentence.
 */
export function computeCapabilitySummary(
  toggleValues: Record<string, boolean>,
  requirements: CapabilityRequirement[],
): CapabilitySummary {
  const missingLabels = new Set<string>();
  const capabilities = requirements.map<CapabilityStatus>((requirement) => {
    const missing = requirement.requiredToggleKeys.filter((key) => toggleValues[key] !== true);
    const labels = missing.map(humanizeToggleKey);
    labels.forEach((label) => missingLabels.add(label));

    return {
      key: requirement.capabilityKey,
      label: requirement.label,
      available: missing.length === 0,
      note: missing.length === 0 ? null : `Requires ${joinLabels(labels)}.`,
    };
  });

  const available = capabilities.filter((capability) => capability.available);
  const unavailable = capabilities.filter((capability) => !capability.available);
  const clauses: string[] = [];

  if (available.length > 0) {
    clauses.push(
      `${joinLabels(available.map((capability) => capability.label))} ${
        available.length === 1 ? "is" : "are"
      } available.`,
    );
  }

  if (unavailable.length > 0) {
    clauses.push(
      `${joinLabels(unavailable.map((capability) => capability.label))} will not be available — enable ${joinLabels([...missingLabels])} to unlock ${
        unavailable.length === 1 ? "it" : "them"
      }.`,
    );
  }

  return { capabilities, summarySentence: clauses.join(" ") };
}
