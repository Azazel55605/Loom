import type * as React from "react";

import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";

import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { RadioGroup } from "@loom/ui-kit/components/ui/radio-group";
import { GENERIC_ICONS } from "@loom/ui-kit/lib/generic-icons";
import { cn } from "@loom/ui-kit/lib/utils";

const USE_DEFAULT = "__default__";

/**
 * Accessible single-select picker for Loom's curated generic icon references.
 * `null` is a first-class "use default" choice so callers can both assign and
 * clear an icon without inventing separate reset controls.
 */
export function GenericIconPicker({
  value,
  defaultIcon,
  label,
  defaultLabel = "Use default",
  onChange,
  disabled,
}: {
  value: string | null;
  defaultIcon: string | null;
  label: string;
  defaultLabel?: string;
  onChange: (value: string | null) => void;
  disabled?: boolean;
}) {
  const unrepresentable =
    value !== null && !GENERIC_ICONS.some((icon) => `lucide:${icon.name}` === value) ? value : null;

  return (
    <RadioGroup
      aria-label={label}
      value={value ?? USE_DEFAULT}
      onValueChange={(next) => onChange(next === USE_DEFAULT ? null : next)}
      disabled={disabled}
      className="flex flex-wrap gap-2"
    >
      <IconTile value={USE_DEFAULT} label={defaultLabel} className="border-dashed">
        <ConnectorIcon typeIcon={defaultIcon} iconOverride={null} size={20} />
      </IconTile>

      {unrepresentable !== null && (
        <IconTile value={unrepresentable} label={`Current icon (${unrepresentable})`}>
          <ConnectorIcon typeIcon={null} iconOverride={unrepresentable} size={20} />
        </IconTile>
      )}

      {GENERIC_ICONS.map((icon) => (
        <IconTile key={icon.name} value={`lucide:${icon.name}`} label={icon.label}>
          <ConnectorIcon typeIcon={null} iconOverride={`lucide:${icon.name}`} size={20} />
        </IconTile>
      ))}
    </RadioGroup>
  );
}

function IconTile({
  value,
  label,
  className,
  children,
}: {
  value: string;
  label: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <RadioGroupPrimitive.Item
      value={value}
      title={label}
      aria-label={label}
      className={cn(
        "surface-panel inline-flex h-10 w-10 items-center justify-center rounded-md border transition-colors",
        "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
        "disabled:cursor-not-allowed disabled:opacity-50 hover:border-accent/60",
        "data-[state=checked]:border-accent data-[state=checked]:ring-1 data-[state=checked]:ring-accent",
        className,
      )}
    >
      {children}
    </RadioGroupPrimitive.Item>
  );
}
