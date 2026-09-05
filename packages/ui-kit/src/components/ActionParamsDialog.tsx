import * as React from "react";
import { Loader2 } from "lucide-react";

import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import {
  SchemaForm,
  validateSchemaValues,
  type JsonSchema,
} from "@loom/ui-kit/components/SchemaForm";
import type { ConnectorAction } from "@loom/ui-kit/lib/api";

/** Whether an action's schema declares any parameters at all. */
export function takesParameters(action: ConnectorAction): boolean {
  const schema = action.paramsSchema;
  if (typeof schema !== "object" || schema === null) return false;
  const properties = (schema as JsonSchema).properties;
  return properties !== undefined && Object.keys(properties).length > 0;
}

/**
 * Collects an action's parameters before running it.
 *
 * Generated from the action's own `paramsSchema` through the same `SchemaForm`
 * the add-connector dialog uses, so an action gaining a parameter needs no
 * frontend change. Same subset limitation applies — string, number, and boolean
 * only; see `SchemaForm`.
 *
 * Extracted from `ConnectorCard` so the `Button` widget shares it rather than
 * growing a second parameter prompt: two dialogs collecting the same schema
 * would drift, and the widget one would be the one nobody remembered to fix.
 * `action === null` renders nothing, so a caller can keep it mounted
 * unconditionally and let the selected action drive it.
 */
export function ActionParamsDialog({
  action,
  connectorName,
  isPending,
  submitLabel = "Run",
  onOpenChange,
  onSubmit,
}: {
  action: ConnectorAction | null;
  connectorName: string;
  isPending: boolean;
  /**
   * What the confirm button says. Defaults to running the action, which is what
   * every caller but one wants.
   *
   * `PlacementActionEditor` is that one: it collects the same parameters
   * against the same schema, but stores them on a tile instead of dispatching
   * them, and a button labelled "Run" there would promise something the dialog
   * does not do.
   */
  submitLabel?: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (params: Record<string, unknown>) => void;
}) {
  const [values, setValues] = React.useState<Record<string, unknown>>({});
  const [errors, setErrors] = React.useState<Record<string, string>>({});

  if (action === null) return null;

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {action.label} — {connectorName}
          </DialogTitle>
          <DialogDescription>
            {action.description ?? "This action takes parameters."}
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-4"
          // See ConnectorInstanceDialog: native validation bubbles are a
          // browser-default control, and they would pre-empt the connector's
          // own `invalidParams` message.
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            const found = validateSchemaValues(action.paramsSchema, values);
            setErrors(found);
            if (Object.keys(found).length === 0) onSubmit(values);
          }}
        >
          <SchemaForm
            schema={action.paramsSchema}
            value={values}
            onChange={setValues}
            errors={errors}
            disabled={isPending}
            idPrefix={`action-${action.id}`}
          />

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isPending}>
              {isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              {submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
