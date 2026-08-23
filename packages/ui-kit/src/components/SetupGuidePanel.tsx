import { Clipboard } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import type { SetupGuide } from "@loom/ui-kit/lib/api";

const PLACEHOLDER = /\{\{([A-Za-z0-9_.-]+)\}\}/g;

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

/** Applies the setup-guide placeholder contract without mutating form values. */
export function renderSetupGuideTemplate(
  template: string,
  formValues: Record<string, unknown>,
): string {
  return template.replace(PLACEHOLDER, (_placeholder, fieldName: string) =>
    templateValue(valueAtPath(formValues, fieldName), fieldName),
  );
}

/**
 * Type-provided setup instructions with live values from the generated form.
 * Templates remain plain text: the connector supplies no HTML and the client
 * performs only the documented `{{fieldName}}` substitution.
 */
export function SetupGuidePanel({
  guide,
  formValues,
}: {
  guide: SetupGuide;
  formValues: Record<string, unknown>;
}) {
  const rendered = renderSetupGuideTemplate(guide.template, formValues);

  async function copy() {
    try {
      await navigator.clipboard.writeText(rendered);
      toast.success("Setup guide copied.");
    } catch {
      toast.error("Could not copy the setup guide.");
    }
  }

  return (
    <Card className="surface-panel h-fit">
      <CardHeader className="gap-1 p-4">
        <CardTitle className="text-base">Setup guide</CardTitle>
        <CardDescription>{guide.description}</CardDescription>
      </CardHeader>
      <CardContent className="px-4 pb-4">
        <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-muted/40 p-3 text-xs">
          <code>{rendered}</code>
        </pre>
      </CardContent>
      <CardFooter className="justify-end px-4 pb-4">
        <Button type="button" variant="outline" size="sm" onClick={() => void copy()}>
          <Clipboard data-icon="inline-start" aria-hidden="true" />
          Copy
        </Button>
      </CardFooter>
    </Card>
  );
}
