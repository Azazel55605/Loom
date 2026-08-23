import type * as React from "react";

import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";

import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { RadioGroup } from "@loom/ui-kit/components/ui/radio-group";
import { GENERIC_ICONS } from "@loom/ui-kit/lib/generic-icons";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Picks a connector instance's icon override from the curated generic set.
 *
 * Built on Radix `RadioGroup` reskinned as tiles, the same move
 * `SegmentedControl` makes: exactly one option is selected, arrow keys should
 * move between them, and the whole grid should be a single tab stop. (This is
 * the "second single-select group" `ColorSwatchPicker` anticipated when it
 * chose `aria-pressed` buttons rather than pull the primitive in for one
 * control — the primitive is in the tree now, so this one uses it.)
 *
 * ## "Use default" is an option, not an absence
 *
 * Clearing the override is a choice a person makes, so it is a tile they can
 * land on with an arrow key rather than a "reset" link parked somewhere else.
 * It previews what default actually resolves to — the connector *type*'s own
 * icon, drawn through the same `ConnectorIcon` — because "default" means
 * nothing until you can see it.
 *
 * Radix requires a string value, so the sentinel below stands in for `null` on
 * the wire. It cannot collide with a real value: every real one is prefixed.
 *
 * ## An override the grid cannot represent
 *
 * The picker only ever *sets* generic icons, but the stored value can be
 * something else — set through the API directly, or chosen from a larger set by
 * a newer client. A controlled Radix value with no matching item leaves the
 * whole group with nothing selected, which reads as "this connector has no
 * icon" when it plainly has one, and quietly invites the user to overwrite a
 * choice they could not see. So a value outside the grid gets a tile of its
 * own, selected, showing what it actually resolves to.
 *
 * ## Only generic icons are offered
 *
 * Brand icons are vendored per connector *type* and identify a product; letting
 * a user label their reverse proxy with someone else's logo is how an icon set
 * stops meaning anything. The override exists to tell two instances of the same
 * type apart, which the generic set does.
 */

/** Stands in for `null` inside the radio group. Unprefixed, so it can never be
 *  confused with a `brand:`/`lucide:` reference. */
const USE_DEFAULT = "__default__";

export function ConnectorIconPicker({
  value,
  typeIcon,
  onChange,
  disabled,
}: {
  /** The current override, or `null` for "use the type's own icon". */
  value: string | null;
  /** `metadata.icon` — what "use default" resolves to, for the preview tile. */
  typeIcon: string | null;
  onChange: (value: string | null) => void;
  disabled?: boolean;
}) {
  // Anything set but not offered below. Kept as its own tile rather than
  // silently dropped — see the note above.
  const unrepresentable =
    value !== null && !GENERIC_ICONS.some((icon) => `lucide:${icon.name}` === value) ? value : null;

  return (
    <RadioGroup
      aria-label="Connector icon"
      value={value ?? USE_DEFAULT}
      onValueChange={(next) => onChange(next === USE_DEFAULT ? null : next)}
      disabled={disabled}
      className="flex flex-wrap gap-2"
    >
      {/* Dashed, because the preview inside it is the *type's* icon and may
          well be a member of the set below — a debug connector's default tile
          and its "Debug" tile draw the same bug. The border is what says one
          of them means "inherit". */}
      <IconTile value={USE_DEFAULT} label="Use default" className="border-dashed">
        <ConnectorIcon typeIcon={typeIcon} iconOverride={null} size={20} />
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

/** One tile. The label is the accessible name and the tooltip; it is not drawn,
 *  because sixteen captions in a grid this size is a wall of text. */
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
        "disabled:cursor-not-allowed disabled:opacity-50",
        "hover:border-accent/60",
        // Selected reads as the accent ring rather than a fill, so the icon
        // inside stays legible and a brand logo is not sitting on a coloured
        // block that fights its own palette.
        "data-[state=checked]:border-accent data-[state=checked]:ring-1 data-[state=checked]:ring-accent",
        className,
      )}
    >
      {children}
    </RadioGroupPrimitive.Item>
  );
}
