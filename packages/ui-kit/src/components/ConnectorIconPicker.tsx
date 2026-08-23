import { GenericIconPicker } from "@loom/ui-kit/components/GenericIconPicker";

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
  return (
    <GenericIconPicker
      label="Connector icon"
      value={value}
      defaultIcon={typeIcon}
      defaultLabel="Use connector default"
      onChange={onChange}
      disabled={disabled}
    />
  );
}
