import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { ConnectorIconPicker } from "@loom/ui-kit/components/ConnectorIconPicker";
import { SetupGuidePanel } from "@loom/ui-kit/components/SetupGuidePanel";
import {
  defaultsForSchema,
  SchemaForm,
  validateSchemaValues,
} from "@loom/ui-kit/components/SchemaForm";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type { ConnectorInstanceSummary } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Add or reconfigure a connector instance.
 *
 * One dialog for both, as with groups and users: the fields, the validation and
 * the generated form are identical, and the only difference is which request is
 * sent. Two dialogs would be one form duplicated, and the copy that gets edited
 * less is the one that drifts.
 *
 * **Nothing here knows what a connector type is.** The type list comes from
 * `GET /connector-types` and the configuration form is generated from each
 * type's published `configSchema`, so a real integration registered on the
 * backend appears in this dialog with no frontend change at all. Special-casing
 * `"debug"` — the only type that exists today — would defeat the entire point
 * of the registry.
 */
export function ConnectorInstanceDialog({
  open,
  instance,
  onOpenChange,
  onSaved,
}: {
  open: boolean;
  /** The instance being edited, or `null` to add a new one. */
  instance: ConnectorInstanceSummary | null;
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<void>;
}) {
  const api = useApiClient();
  const isEditing = instance !== null;

  const types = useQuery({
    queryKey: ["connector-types"],
    queryFn: ({ signal }) => api.getConnectorTypes(signal),
    enabled: open,
    // Code-defined and identical for a given build, so it changes only on a
    // redeploy. Refetching it within a session buys nothing.
    staleTime: Infinity,
    retry: false,
  });

  // Editing loads the instance's own detail for its stored config; the type is
  // fixed and cannot be changed (the backend has no route for it — a different
  // type is a different connector).
  const detail = useQuery({
    queryKey: ["connector-instance", instance?.id],
    queryFn: ({ signal }) => api.getConnectorInstance(instance!.id, signal),
    enabled: open && isEditing,
    retry: false,
  });

  const [typeId, setTypeId] = React.useState<string>(instance?.connectorType ?? "");
  const [name, setName] = React.useState<string>(instance?.name ?? "");
  const [config, setConfig] = React.useState<Record<string, unknown>>({});
  // `null` is "no override", which is a real value here rather than "unset" —
  // see `ConnectorIconPicker`.
  const [iconOverride, setIconOverride] = React.useState<string | null>(
    instance?.iconOverride ?? null,
  );
  const [errors, setErrors] = React.useState<Record<string, string>>({});
  const [nameError, setNameError] = React.useState<string | null>(null);
  const [failure, setFailure] = React.useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  const selectedType = types.data?.find((candidate) => candidate.typeId === typeId) ?? null;
  const setupGuide = !isEditing ? (selectedType?.setupGuide ?? null) : null;

  // Seed the config form once the schema is known: on create from the schema's
  // own defaults, on edit from what is actually stored. Keyed on the schema and
  // the loaded detail rather than run on every render, so typing is not undone.
  const seededFor = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!open) {
      seededFor.current = null;
      return;
    }
    if (selectedType === null) return;
    if (isEditing && !detail.isSuccess) return;

    const key = `${typeId}:${isEditing ? detail.dataUpdatedAt : "new"}`;
    if (seededFor.current === key) return;
    seededFor.current = key;

    const stored = detail.data?.config;
    setConfig(
      isEditing && typeof stored === "object" && stored !== null
        ? { ...(stored as Record<string, unknown>) }
        : defaultsForSchema(selectedType.configSchema),
    );
    setErrors({});
  }, [open, selectedType, typeId, isEditing, detail.isSuccess, detail.data, detail.dataUpdatedAt]);

  React.useEffect(() => {
    if (open) {
      setFailure(null);
      setNameError(null);
      // Re-seeded on open rather than only at mount: this dialog is kept
      // mounted and handed a different `instance` each time, so state
      // initialised from props once would show the previously edited
      // connector's icon.
      setIconOverride(instance?.iconOverride ?? null);
    }
  }, [open, instance]);

  // With exactly one registered type there is nothing to choose, so choosing it
  // for the user removes a pointless step. Still driven by the list, never by a
  // hardcoded `"debug"`.
  React.useEffect(() => {
    if (!open || isEditing || typeId !== "") return;
    if (types.data?.length === 1) setTypeId(types.data[0].typeId);
  }, [open, isEditing, typeId, types.data]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setFailure(null);

    const trimmed = name.trim();
    if (trimmed === "") {
      setNameError("Give the connector a name.");
      return;
    }
    setNameError(null);

    if (selectedType === null) {
      setFailure("Choose a connector type.");
      return;
    }

    // Local checks are `required` and basic types only. The connector's factory
    // on the backend is the real validator, and its refusal is reported below
    // as a form-level Alert rather than a toast — it names the field to fix.
    const found = validateSchemaValues(selectedType.configSchema, config);
    setErrors(found);
    if (Object.keys(found).length > 0) return;

    setIsSubmitting(true);
    try {
      if (isEditing) {
        // Always sent when editing, `null` included: omitting the key means
        // "leave it alone", which would make clearing an override impossible.
        await api.updateConnectorInstance(instance.id, {
          name: trimmed,
          config,
          iconOverride,
        });
      } else {
        await api.createConnectorInstance({
          connectorType: selectedType.typeId,
          name: trimmed,
          config,
        });
      }
    } catch (error: unknown) {
      setFailure(describeAdminFailure(error).message);
      return;
    } finally {
      setIsSubmitting(false);
    }

    toast.success(isEditing ? `Updated ${trimmed}.` : `Added ${trimmed}.`);
    await onSaved();
    onOpenChange(false);
  }

  const isLoading = types.isPending || (isEditing && detail.isPending);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className={cn(
          "max-h-[85vh] overflow-y-auto",
          setupGuide !== null && "sm:max-w-3xl",
        )}
      >
        <DialogHeader>
          <DialogTitle>
            {isEditing ? `Edit ${instance.name}` : "Add connector"}
          </DialogTitle>
          <DialogDescription>
            {isEditing
              ? "The connector type cannot be changed — a different type is a different connector. Configuration is replaced with exactly what is here."
              : "Pick a type, name the instance, and fill in whatever configuration it asks for."}
          </DialogDescription>
        </DialogHeader>

        {/* `noValidate`: `min`/`max` from the schema reach the input so the
            stepper knows its bounds, but the browser's own validation bubble
            must not be what refuses a submission. It is an unstyled native
            popup — the exact browser-default control docs/UI_GUIDELINES.md
            rules out — and worse, it would block the request whose 400 carries
            the connector's own, better-worded objection. Our checks and the
            backend's are the two that get to speak here. */}
        <form className="space-y-4" noValidate onSubmit={submit}>
          {failure !== null && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" aria-hidden="true" />
              <AlertTitle>
                {isEditing ? "Could not save the connector" : "Could not add the connector"}
              </AlertTitle>
              {/* The backend's own words. A 400 here is usually the connector
                  rejecting the configuration and naming the field, which a
                  generic toast would throw away. */}
              <AlertDescription>{failure}</AlertDescription>
            </Alert>
          )}

          {types.isError && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" aria-hidden="true" />
              <AlertTitle>Could not load connector types</AlertTitle>
              <AlertDescription>{describeAdminFailure(types.error).message}</AlertDescription>
            </Alert>
          )}

          <div className="space-y-1.5">
            <Label htmlFor="connector-type">Type</Label>
            {isLoading ? (
              <Skeleton className="h-9 w-full" />
            ) : (
              <Select
                value={typeId}
                onValueChange={(next) => {
                  setTypeId(next);
                  seededFor.current = null;
                }}
                disabled={isEditing || isSubmitting}
              >
                <SelectTrigger id="connector-type">
                  <SelectValue placeholder="Choose a connector type" />
                </SelectTrigger>
                <SelectContent>
                  {types.data?.map((candidate) => (
                    <SelectItem key={candidate.typeId} value={candidate.typeId}>
                      <span className="flex items-center gap-2">
                        <ConnectorIcon typeIcon={candidate.icon} iconOverride={null} size={16} />
                        {candidate.displayName}
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="connector-name">Name</Label>
            <Input
              id="connector-name"
              autoFocus={!isEditing}
              placeholder="Media server"
              disabled={isSubmitting}
              aria-invalid={nameError !== null}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              What you call this instance. Separate from its configuration.
            </p>
            {nameError !== null && (
              <p className="text-xs font-medium text-destructive">{nameError}</p>
            )}
          </div>

          {/* Editing only. A brand-new instance has nothing to be told apart
              from yet, and `POST /connector-instances` deliberately takes no
              `iconOverride` — one field with one place to set it is one fewer
              way for the two to disagree. */}
          {isEditing && !isLoading && (
            <div className="space-y-2 border-t border-border pt-4">
              {/* Visual heading only — no `htmlFor`, because the control it
                  labels is a radio group of sixteen tiles rather than one
                  focusable input. The group carries its own `aria-label`. */}
              <p className="text-sm font-medium">Icon</p>
              <p className="text-xs text-muted-foreground">
                Overrides the icon this connector type ships with — useful when
                you run more than one of the same thing.
              </p>
              <ConnectorIconPicker
                value={iconOverride}
                typeIcon={instance.metadata.icon}
                onChange={setIconOverride}
                disabled={isSubmitting}
              />
            </div>
          )}

          {selectedType !== null && !isLoading && (
            <div className="space-y-2 border-t border-border pt-4">
              <p className="text-sm font-medium">{selectedType.displayName} configuration</p>
              <div
                className={cn(
                  "grid gap-4",
                  setupGuide !== null && "lg:grid-cols-2 lg:items-start",
                )}
              >
                <SchemaForm
                  schema={selectedType.configSchema}
                  value={config}
                  onChange={setConfig}
                  errors={errors}
                  disabled={isSubmitting}
                  idPrefix={`config-${selectedType.typeId}`}
                />
                {setupGuide !== null ? (
                  <SetupGuidePanel guide={setupGuide} formValues={config} />
                ) : null}
              </div>
            </div>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={isSubmitting}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting || selectedType === null}>
              {isSubmitting && <Loader2 className="animate-spin" aria-hidden="true" />}
              {isEditing ? "Save" : "Add connector"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
