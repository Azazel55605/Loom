import * as React from "react";

import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { Search } from "lucide-react";

import { AppIcon } from "@loom/ui-kit/components/AppIcon";
import { Input } from "@loom/ui-kit/components/ui/input";
import { RadioGroup } from "@loom/ui-kit/components/ui/radio-group";
import { iconCatalogEntry, searchIconSections } from "@loom/ui-kit/lib/icon-catalog";
import { cn } from "@loom/ui-kit/lib/utils";

/** Stands in for `null` inside the radio group. Unprefixed, so it can never be
 *  confused with a real reference. */
const USE_DEFAULT = "__default__";

/**
 * The one icon picker: every source the resolver knows — the curated lucide
 * set ("Default"), the vendored connector brands, the curated Tabler subset,
 * and the vendored Simple Icons — behind a single search box.
 *
 * ## Layout
 *
 * Labelled sections in one scroll area rather than tabs, because search runs
 * across all four sources at once and tabs would hide most of the matches
 * behind a click. A section with no matches is removed, not shown empty. Tiles
 * are the same 40px bordered squares as `ColorSwatchPicker`'s swatches, with an
 * accent ring (never a fill) for the selection.
 *
 * ## One radio group
 *
 * Radix `RadioGroup` reskinned as tiles across all sections: exactly one
 * selection, arrow keys move between tiles, and the whole grid is a single tab
 * stop after the search field.
 *
 * ## "Use default" is an option, not an absence
 *
 * Clearing an icon is a choice, so it is a tile, pinned above the sections and
 * never filtered out. It previews what default resolves to (`defaultIcon`,
 * drawn through the same `AppIcon`) because "default" means nothing until you
 * can see it. `null` maps to a sentinel since Radix values are strings.
 *
 * ## A value the catalog cannot represent
 *
 * The stored value can name something this build lacks — set through the API,
 * or by a newer client with a larger set. A controlled Radix value with no
 * matching item selects nothing, which reads as "no icon" and invites the user
 * to overwrite a choice they cannot see; so such a value gets a selected tile
 * of its own showing what it actually resolves to.
 *
 * ## Brand marks are offered
 *
 * Earlier pickers deliberately offered generic icons only. Users now choose
 * logos too, for connector overrides, button tiles, groups and folders — a
 * reverse-proxy button with the proxy's own mark is exactly the use. The marks
 * are still shown for identification only; see the disclaimer in
 * `docs/THIRD_PARTY_ICONS.md`.
 */
export function IconPicker({
  value,
  onChange,
  label = "Icon",
  defaultIcon = null,
  defaultLabel = "Use default",
  disabled,
}: {
  /** The chosen reference, or `null` for "use the default". */
  value: string | null;
  onChange: (value: string | null) => void;
  /** Accessible name of the radio group. */
  label?: string;
  /** What `null` resolves to, previewed on the default tile. */
  defaultIcon?: string | null;
  defaultLabel?: string;
  disabled?: boolean;
}) {
  const [query, setQuery] = React.useState("");
  const sections = React.useMemo(() => searchIconSections(query), [query]);
  const unrepresentable = value !== null && iconCatalogEntry(value) === undefined ? value : null;
  const idPrefix = React.useId();

  return (
    <div className="flex flex-col gap-3">
      <div className="relative">
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          type="text"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search icons"
          aria-label={`Search ${label.toLowerCase()}`}
          disabled={disabled}
          className="pl-8"
        />
      </div>

      <RadioGroup
        aria-label={label}
        value={value ?? USE_DEFAULT}
        onValueChange={(next) => onChange(next === USE_DEFAULT ? null : next)}
        disabled={disabled}
        className="flex max-h-72 flex-col gap-4 overflow-y-auto pr-1"
      >
        <div className="flex flex-wrap gap-2">
          <IconTile value={USE_DEFAULT} label={defaultLabel} className="border-dashed">
            <AppIcon icon={defaultIcon} />
          </IconTile>
          {unrepresentable !== null && (
            <IconTile value={unrepresentable} label={`Current icon (${unrepresentable})`}>
              <AppIcon icon={unrepresentable} fallback={defaultIcon} />
            </IconTile>
          )}
        </div>

        {sections.map((section) => {
          const headingId = `${idPrefix}-${section.source}`;
          return (
            <div key={section.source} role="group" aria-labelledby={headingId} className="flex flex-col gap-2">
              <p id={headingId} className="text-xs font-medium text-muted-foreground">
                {section.title}{" "}
                <span className="tabular-nums opacity-70">{section.entries.length}</span>
              </p>
              <div className="flex flex-wrap gap-2">
                {section.entries.map((entry) => (
                  <IconTile key={entry.reference} value={entry.reference} label={entry.label}>
                    <AppIcon icon={entry.reference} />
                  </IconTile>
                ))}
              </div>
            </div>
          );
        })}

        {sections.length === 0 && (
          <p className="text-sm text-muted-foreground">No icons match “{query.trim()}”.</p>
        )}
      </RadioGroup>
    </div>
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
        "surface-panel inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-md border transition-colors",
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
