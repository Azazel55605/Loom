import { DiscoveryFieldPicker } from "@loom/ui-kit/components/DiscoveryFieldPicker";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import { cn } from "@loom/ui-kit/lib/utils";
import type { DiscoveredResource } from "@loom/ui-kit/lib/api";

/**
 * A form generated from a JSON Schema document.
 *
 * This exists so that adding a connector *type* to the backend needs no
 * frontend change at all: `GET /connector-types` publishes each type's
 * `configSchema`, and the add/edit dialog renders whatever it finds there. The
 * alternative — a hand-written form per connector — is the thing the whole
 * registry design is meant to avoid, and it would put a UI change on the
 * critical path of every new integration.
 *
 * ## Known limitation: this supports a deliberate subset of JSON Schema
 *
 * Rendered: `string`, `number`, `integer`, `boolean`, and nested `object`
 * properties (recursively, as an indented group).
 *
 * **Not** rendered, and skipped with a visible note rather than silently
 * dropped: `enum`, `array`, `oneOf`/`anyOf`/`allOf`, `$ref`, tuple types, and
 * any property whose `type` is absent or unrecognised. That is enough for the
 * currently registered Debug and Docker schemas, and it is a known future
 * extension rather than an oversight. The visible note matters:
 * a field quietly missing from a form is a configuration a user cannot set and
 * cannot see that they cannot set.
 *
 * Validation here covers **`required` and basic types only**. It is not a
 * JSON Schema validator and must not be mistaken for one — the authoritative
 * check is the connector's own factory on the backend, which rejects
 * configurations this form would happily submit (an out-of-range number, a
 * mutually exclusive pair). Callers must surface that 400; see
 * `ConnectorInstanceDialog`.
 */

/** The subset of JSON Schema this component reads. Everything else is ignored
 *  rather than typed, since an unknown keyword is data we pass over. */
export type JsonSchema = {
  type?: string;
  title?: string;
  description?: string;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  default?: unknown;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  enum?: unknown[];
  /** Loom extension: encrypt this top-level connector-config string at rest. */
  "x-loom-sensitive"?: boolean;
};

export type SchemaFormDiscovery = {
  /** Dotted schema-property path rendered with discovery assistance. */
  targetField: string;
  canDiscover: (currentValues: Record<string, unknown>) => boolean;
  onDiscover: (currentValues: Record<string, unknown>) => Promise<DiscoveredResource[]>;
};

/** What a schema property was turned into, or why it was not. */
type RenderableKind = "string" | "number" | "boolean" | "object" | "unsupported";

function kindOf(schema: JsonSchema): RenderableKind {
  // An enum is a closed set and wants a Select, not a free-text box. Rendering
  // it as one would let a user type a value the backend must then reject, so it
  // is declared unsupported until the control exists.
  if (Array.isArray(schema.enum)) return "unsupported";

  switch (schema.type) {
    case "string":
      return "string";
    case "number":
    case "integer":
      return "number";
    case "boolean":
      return "boolean";
    case "object":
      return "object";
    default:
      return "unsupported";
  }
}

/** Reads the value at `path` out of a nested plain object. */
function valueAt(root: unknown, path: string[]): unknown {
  let current: unknown = root;
  for (const key of path) {
    if (typeof current !== "object" || current === null) return undefined;
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

/**
 * Returns a copy of `root` with `path` set to `value`, or with the leaf removed
 * when `value` is `undefined`.
 *
 * Removal rather than an explicit `undefined` because the two are different on
 * the wire: `JSON.stringify` drops an `undefined` property, but a key held at
 * `null` would be sent, and a schema with `additionalProperties: false` plus a
 * connector that reads "absent" as "use the default" treats those differently.
 * Clearing an optional field must actually clear it.
 */
function withValueAt(root: unknown, path: string[], value: unknown): Record<string, unknown> {
  const [head, ...rest] = path;
  const base: Record<string, unknown> =
    typeof root === "object" && root !== null ? { ...(root as Record<string, unknown>) } : {};

  if (rest.length === 0) {
    if (value === undefined) {
      delete base[head];
    } else {
      base[head] = value;
    }
    return base;
  }

  base[head] = withValueAt(base[head], rest, value);
  return base;
}

/**
 * The starting values for a schema: every property's `default`, where it
 * declares one.
 *
 * Pre-filling from `default` rather than leaving blanks is what makes a
 * generated form usable — the debug connector's schema declares a default for
 * almost every field, and an empty form would ask the user to invent values the
 * connector already has opinions about.
 */
export function defaultsForSchema(schema: JsonSchema | unknown): Record<string, unknown> {
  const parsed = asSchema(schema);
  const values: Record<string, unknown> = {};

  for (const [key, property] of Object.entries(parsed.properties ?? {})) {
    const kind = kindOf(property);
    if (kind === "object") {
      const nested = defaultsForSchema(property);
      if (Object.keys(nested).length > 0) values[key] = nested;
      continue;
    }
    if (kind === "unsupported") continue;
    if (property.default !== undefined) values[key] = property.default;
  }

  return values;
}

/**
 * `required` and type checking, keyed by dotted path so a caller can map an
 * error back to the field that produced it.
 *
 * Returns an empty object when the values pass. **Not a JSON Schema
 * validator** — see the component doc.
 */
export function validateSchemaValues(
  schema: JsonSchema | unknown,
  values: unknown,
  sensitiveFieldsSet: readonly string[] = [],
): Record<string, string> {
  return validateSchemaFields(asSchema(schema), values, [], new Set(sensitiveFieldsSet));
}

function validateSchemaFields(
  schema: JsonSchema,
  values: unknown,
  prefix: string[],
  sensitiveFieldsSet: ReadonlySet<string>,
): Record<string, string> {
  const parsed = asSchema(schema);
  const errors: Record<string, string> = {};
  const required = new Set(parsed.required ?? []);

  for (const [key, property] of Object.entries(parsed.properties ?? {})) {
    const kind = kindOf(property);
    if (kind === "unsupported") continue;

    const path = [...prefix, key];
    const value = valueAt(values, path);

    if (kind === "object") {
      Object.assign(errors, validateSchemaFields(property, values, path, sensitiveFieldsSet));
      continue;
    }

    const missing = value === undefined || value === "" || value === null;
    if (missing) {
      const dotted = path.join(".");
      if (required.has(key) && !sensitiveFieldsSet.has(dotted)) {
        errors[dotted] = "This field is required.";
      }
      continue;
    }

    if (kind === "number" && typeof value !== "number") {
      errors[path.join(".")] = "Enter a number.";
    }
    if (kind === "string" && typeof value !== "string") {
      errors[path.join(".")] = "Enter a value.";
    }
  }

  return errors;
}

/** Top-level keys using Loom's sensitive-field schema extension. */
export function sensitiveFieldKeysForSchema(schema: JsonSchema | unknown): string[] {
  return Object.entries(asSchema(schema).properties ?? {})
    .filter(([, property]) => property["x-loom-sensitive"] === true)
    .map(([key]) => key);
}

function asSchema(schema: JsonSchema | unknown): JsonSchema {
  return typeof schema === "object" && schema !== null ? (schema as JsonSchema) : {};
}

export function SchemaForm({
  schema,
  value,
  onChange,
  errors,
  disabled,
  idPrefix = "schema",
  discovery,
  sensitiveFieldsSet = [],
}: {
  /** The type's `configSchema`, as published by `GET /connector-types`. */
  schema: JsonSchema | unknown;
  /** The current values, as a plain object. Controlled. */
  value: Record<string, unknown>;
  onChange: (next: Record<string, unknown>) => void;
  /** Messages keyed by dotted path, as returned by `validateSchemaValues`. */
  errors?: Record<string, string>;
  disabled?: boolean;
  /** Prefix for generated input ids, so two forms on one page do not collide. */
  idPrefix?: string;
  /** Optional generic discovery assistance for one schema field. */
  discovery?: SchemaFormDiscovery;
  /** Existing secrets reported by an edit response; their values stay blank. */
  sensitiveFieldsSet?: readonly string[];
}) {
  return (
    <SchemaFields
      schema={asSchema(schema)}
      root={value}
      path={[]}
      onChange={onChange}
      errors={errors ?? {}}
      disabled={disabled === true}
      idPrefix={idPrefix}
      discovery={discovery}
      sensitiveFieldsSet={new Set(sensitiveFieldsSet)}
    />
  );
}

function SchemaFields({
  schema,
  root,
  path,
  onChange,
  errors,
  disabled,
  idPrefix,
  discovery,
  sensitiveFieldsSet,
}: {
  schema: JsonSchema;
  root: Record<string, unknown>;
  path: string[];
  onChange: (next: Record<string, unknown>) => void;
  errors: Record<string, string>;
  disabled: boolean;
  idPrefix: string;
  discovery?: SchemaFormDiscovery;
  sensitiveFieldsSet: ReadonlySet<string>;
}) {
  const properties = Object.entries(schema.properties ?? {});
  const required = new Set(schema.required ?? []);

  if (properties.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        This connector takes no configuration.
      </p>
    );
  }

  return (
    <div className="space-y-4">
      {properties.map(([key, property]) => {
        const fieldPath = [...path, key];
        const dotted = fieldPath.join(".");
        const id = `${idPrefix}-${dotted.replace(/\./g, "-")}`;
        const kind = kindOf(property);
        const label = property.title ?? key;
        const error = errors[dotted];
        const descriptionId = property.description !== undefined ? `${id}-description` : undefined;
        const errorId = error !== undefined ? `${id}-error` : undefined;

        if (kind === "unsupported") {
          return (
            <div key={dotted} className="rounded-md border border-dashed border-border p-3">
              <p className="text-sm font-medium">{label}</p>
              <p className="text-xs text-muted-foreground">
                This field&rsquo;s type is not supported by the generated form yet, so it
                is left at whatever the connector defaults to. Shown rather than hidden so
                the gap is visible.
              </p>
            </div>
          );
        }

        if (kind === "object") {
          return (
            <fieldset key={dotted} className="rounded-md border border-border p-3">
              <legend className="px-1 text-sm font-medium">{label}</legend>
              {property.description !== undefined && (
                <p className="mb-3 text-xs text-muted-foreground">{property.description}</p>
              )}
              <SchemaFields
                schema={property}
                root={root}
                path={fieldPath}
                onChange={onChange}
                errors={errors}
                disabled={disabled}
                idPrefix={idPrefix}
                discovery={discovery}
                sensitiveFieldsSet={sensitiveFieldsSet}
              />
            </fieldset>
          );
        }

        const current = valueAt(root, fieldPath);
        const set = (next: unknown) => onChange(withValueAt(root, fieldPath, next));
        const isSensitive = property["x-loom-sensitive"] === true;
        const sensitiveValueIsSet = isSensitive && sensitiveFieldsSet.has(dotted);
        const sensitiveStateId = sensitiveValueIsSet ? `${id}-sensitive-state` : undefined;

        if (kind === "boolean") {
          return (
            <div key={dotted} className="flex items-start justify-between gap-4">
              <div className="space-y-0.5">
                <Label htmlFor={id}>{label}</Label>
                {property.description !== undefined && (
                  <p id={descriptionId} className="text-xs text-muted-foreground">
                    {property.description}
                  </p>
                )}
              </div>
              <Switch
                id={id}
                disabled={disabled}
                aria-describedby={descriptionId}
                checked={current === true}
                onCheckedChange={(checked) => set(checked)}
              />
            </div>
          );
        }

        return (
          <div key={dotted} className="space-y-1.5">
            <Label htmlFor={id}>
              {label}
              {required.has(key) && <span aria-hidden="true"> *</span>}
            </Label>
            {kind === "string" && !isSensitive && discovery?.targetField === dotted ? (
              <DiscoveryFieldPicker
                fieldName={label}
                inputId={id}
                currentValue={current}
                disabled={disabled}
                ariaInvalid={error !== undefined}
                ariaDescribedBy={
                  [descriptionId, errorId].filter(Boolean).join(" ") || undefined
                }
                canDiscover={!disabled && discovery.canDiscover(root)}
                onDiscover={() => discovery.onDiscover(root)}
                onSelect={set}
              />
            ) : (
              <Input
                id={id}
                disabled={disabled}
                type={kind === "number" ? "number" : isSensitive ? "password" : "text"}
                autoComplete={isSensitive ? "new-password" : undefined}
                inputMode={kind === "number" ? "decimal" : undefined}
                min={property.minimum}
                max={property.maximum}
                aria-invalid={error !== undefined}
                aria-describedby={
                  [descriptionId, sensitiveStateId, errorId].filter(Boolean).join(" ") || undefined
                }
                value={current === undefined || current === null ? "" : String(current)}
                onChange={(event) => {
                  const raw = event.target.value;
                  if (kind !== "number") {
                    set(raw === "" ? undefined : raw);
                    return;
                  }
                  // An empty box means "not set", not zero. A half-typed value
                  // like "-" or "1e" parses to NaN, which must not be sent — it
                  // serializes to `null` and the backend would reject it with a
                  // message about the wrong field. Keeping the leaf absent until
                  // the text is a real number leaves the user's typing alone and
                  // lets `required` do the complaining.
                  if (raw.trim() === "") {
                    set(undefined);
                    return;
                  }
                  const parsed = Number(raw);
                  set(Number.isFinite(parsed) ? parsed : undefined);
                }}
              />
            )}
            {property.description !== undefined && (
              <p id={descriptionId} className="text-xs text-muted-foreground">
                {property.description}
              </p>
            )}
            {sensitiveValueIsSet && (
              <p id={sensitiveStateId} className="text-xs text-muted-foreground">
                Value is set — leave blank to keep unchanged.
              </p>
            )}
            {error !== undefined && (
              <p id={`${id}-error`} className={cn("text-xs font-medium text-destructive")}>
                {error}
              </p>
            )}
          </div>
        );
      })}
    </div>
  );
}
