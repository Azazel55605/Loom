import { useQuery } from "@tanstack/react-query";

import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import type { PermissionCatalogEntry, PermissionGrant } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";

/**
 * Builds a group's permission grants from the server's catalog.
 *
 * Shared by the create and edit dialogs in the groups panel — the same control
 * in both, because a grant set is a grant set and `PATCH /groups/{id}` replaces
 * it wholesale exactly as `POST /groups` states it.
 *
 * The rows come from `GET /permissions` rather than from a list in this file.
 * Hardcoding the keys would produce a form that silently falls out of date the
 * next time a migration registers one — and since the `permissions` table is
 * the authoritative set, a stale form would be quietly wrong rather than
 * visibly broken.
 *
 * ## Scoping
 *
 * Every grant is global except `connectors.control`, which may be narrowed to a
 * single connector. That is deliberately specific rather than a generic
 * resource-type picker: `connector` is the only resource type that exists in
 * practice today, and a generic picker would be a UI for a concept with exactly
 * one instance — untestable against anything real, and shaped by guesses about
 * the resource types that come later. The schema supports more whenever they
 * arrive.
 */
export function PermissionGrantBuilder({
  catalog,
  value,
  onChange,
}: {
  catalog: PermissionCatalogEntry[];
  value: PermissionGrant[];
  onChange: (value: PermissionGrant[]) => void;
}) {
  const api = useApiClient();
  // Needed only to name the connectors in the scope picker. Allowed to fail:
  // the account editing groups is not guaranteed to hold `connectors.view`, and
  // that should cost the narrow-scope option, not the whole form.
  //
  // The scope id is the **instance** id — a UUID — not the connector type's
  // `metadata.id`. That is what the backend matches a resource-scoped
  // `connectors.control` grant against, and scoping to a type id would produce
  // a grant that silently authorizes nothing.
  const connectors = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    retry: false,
    staleTime: 60_000,
  });

  function grantFor(key: string): PermissionGrant | undefined {
    return value.find((grant) => grant.key === key);
  }

  function setGrant(key: string, grant: PermissionGrant | null): void {
    // Rebuilt in catalog order so the submitted list is stable regardless of
    // the order the boxes were ticked in — which keeps a re-save from looking
    // like a change when nothing changed.
    const next = catalog
      .map((entry) => (entry.key === key ? grant : (grantFor(entry.key) ?? null)))
      .filter((entry): entry is PermissionGrant => entry !== null);
    onChange(next);
  }

  if (catalog.length === 0) {
    return (
      <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
        The permission catalog could not be read, so grants cannot be edited.
      </p>
    );
  }

  return (
    <div className="space-y-3 rounded-md border p-3">
      {catalog.map((entry) => {
        const grant = grantFor(entry.key);
        const granted = grant !== undefined;
        const inputId = `permission-${entry.key}`;
        const scopeable = entry.key === PERMISSION_KEYS.connectorsControl;

        return (
          <div key={entry.key} className="space-y-2">
            <div className="flex items-start gap-3">
              <Checkbox
                id={inputId}
                checked={granted}
                onCheckedChange={(checked) =>
                  setGrant(
                    entry.key,
                    checked === true
                      ? { key: entry.key, resourceType: null, resourceId: null }
                      : null,
                  )
                }
                className="mt-0.5"
              />
              <div className="space-y-0.5 leading-tight">
                <Label htmlFor={inputId} className="cursor-pointer font-mono text-xs">
                  {entry.key}
                </Label>
                <p className="text-xs text-muted-foreground">{entry.description}</p>
              </div>
            </div>

            {scopeable && granted && (
              <div className="ml-7 space-y-1">
                <Select
                  value={grant.resourceId ?? ALL_RESOURCES}
                  onValueChange={(selected) =>
                    setGrant(
                      entry.key,
                      selected === ALL_RESOURCES
                        ? { key: entry.key, resourceType: null, resourceId: null }
                        : {
                            key: entry.key,
                            resourceType: CONNECTOR_RESOURCE_TYPE,
                            resourceId: selected,
                          },
                    )
                  }
                >
                  <SelectTrigger className="w-full sm:w-72" aria-label="Scope">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {/* "All connectors" is the first item of the same select
                        rather than a separate radio pair: the choice is
                        one-of-many either way, and two controls to express one
                        value is two things that can disagree. */}
                    <SelectItem value={ALL_RESOURCES}>All connectors</SelectItem>
                    {connectors.data?.map((instance) => (
                      <SelectItem key={instance.id} value={instance.id}>
                        {instance.name}
                      </SelectItem>
                    ))}
                    {/* A scope that is set but no longer in the list — a
                        connector that was removed, or one this account cannot
                        see. Rendered so the select can display its current
                        value instead of falling back to a blank trigger, which
                        would read as "no scope" for a grant that has one. */}
                    {grant.resourceId !== null &&
                      !(connectors.data ?? []).some(
                        (instance) => instance.id === grant.resourceId,
                      ) && (
                        <SelectItem value={grant.resourceId}>
                          {grant.resourceId} (not listed)
                        </SelectItem>
                      )}
                  </SelectContent>
                </Select>

                {connectors.isError && (
                  <p className="text-xs text-muted-foreground">
                    The connector list could not be read from this account, so
                    only the connectors already scoped here can be chosen.
                  </p>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** Sentinel for the global option. Radix `Select` reserves the empty string, so
 *  "no resource id" needs a value of its own. */
const ALL_RESOURCES = "*";

/** The only resource type the backend scopes against today. Mirrors
 *  `CONNECTOR_RESOURCE_TYPE` in `crates/web-backend/src/routes/connectors.rs`. */
const CONNECTOR_RESOURCE_TYPE = "connector";
