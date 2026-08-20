import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import { Label } from "@loom/ui-kit/components/ui/label";
import type { Group } from "@loom/ui-kit/lib/api";

/**
 * Group membership as a checkbox list.
 *
 * A checkbox list rather than a multi-select popover: membership is small,
 * enumerable, and better read all at once than behind a trigger that summarises
 * it as "3 selected". Radix `Checkbox` underneath, per the no-native-controls
 * rule in docs/UI_GUIDELINES.md.
 *
 * Controlled, and it states the whole selection on every change — which mirrors
 * the API, where `groupIds` replaces membership wholesale rather than applying
 * a delta.
 */
export function GroupMultiSelect({
  groups,
  value,
  onChange,
  disabled = false,
}: {
  groups: Group[];
  value: string[];
  onChange: (value: string[]) => void;
  disabled?: boolean;
}) {
  if (disabled) {
    return (
      <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
        Group membership cannot be changed from this account — reading the group
        list needs the <code className="font-mono text-xs">groups.manage</code>{" "}
        permission. Submitting leaves the current membership as it is.
      </p>
    );
  }

  if (groups.length === 0) {
    return (
      <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
        No groups exist yet. Create one under Settings → Groups.
      </p>
    );
  }

  function toggle(id: string, checked: boolean) {
    // Rebuilt from `groups` rather than by pushing onto `value`, so the result
    // keeps the catalog's order and cannot accumulate duplicates.
    onChange(
      groups
        .map((group) => group.id)
        .filter((groupId) =>
          groupId === id ? checked : value.includes(groupId),
        ),
    );
  }

  return (
    <div className="space-y-2 rounded-md border p-3">
      {groups.map((group) => {
        const inputId = `group-${group.id}`;
        return (
          <div key={group.id} className="flex items-start gap-3">
            <Checkbox
              id={inputId}
              checked={value.includes(group.id)}
              onCheckedChange={(checked) => toggle(group.id, checked === true)}
              className="mt-0.5"
            />
            <div className="space-y-0.5 leading-tight">
              <Label htmlFor={inputId} className="cursor-pointer">
                {group.name}
              </Label>
              {group.description !== null && (
                <p className="text-xs text-muted-foreground">{group.description}</p>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
