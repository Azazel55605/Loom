import * as React from "react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";

export type SearchablePickerOption = {
  id: string;
  label: string;
  /**
   * Optional short tag shown at the end of the row — a sub-target's `kind`,
   * for a picker listing more than one sort of thing.
   *
   * Optional because this list is also the discovery picker, whose options are
   * all the same sort of thing and would gain nothing but noise from a column
   * saying so.
   */
  badge?: string;
};

/** Shared search-and-select list used by discovery and sub-target pickers. */
export function SearchablePickerList({
  options,
  searchLabel,
  emptyMessage = "No options found",
  selectedId,
  disabled = false,
  onSelect,
}: {
  options: SearchablePickerOption[];
  searchLabel: string;
  emptyMessage?: string;
  selectedId?: string | null;
  disabled?: boolean;
  onSelect: (id: string) => void;
}) {
  const [query, setQuery] = React.useState("");
  const deferredQuery = React.useDeferredValue(query);
  const filtered = React.useMemo(() => {
    const needle = deferredQuery.trim().toLocaleLowerCase();
    if (needle === "") return options;
    return options.filter((option) => option.label.toLocaleLowerCase().includes(needle));
  }, [deferredQuery, options]);

  return (
    <div className="flex min-h-0 flex-col gap-3">
      <Input
        aria-label={searchLabel}
        placeholder={searchLabel}
        value={query}
        disabled={disabled}
        onChange={(event) => setQuery(event.target.value)}
      />
      {filtered.length === 0 ? (
        <p className="py-3 text-center text-sm text-muted-foreground">{emptyMessage}</p>
      ) : (
        <div className="flex min-h-0 flex-col gap-1 overflow-y-auto">
          {filtered.map((option) => (
            <Button
              key={option.id}
              type="button"
              variant={option.id === selectedId ? "secondary" : "ghost"}
              // A two-column grid, not a flex row: `minmax(0,1fr)` guarantees the
              // label column can shrink to nothing, so a 70-character sha256
              // option label plus a badge can never push the row — and with it
              // the whole dialog — past its own edge. Flex `min-w-0` did not
              // hold here; this is the shape that does.
              className="grid h-auto w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-2 whitespace-normal text-left"
              disabled={disabled}
              aria-pressed={option.id === selectedId}
              onClick={() => onSelect(option.id)}
            >
              {/* `min-w-0` and no `break-words`: an option label can contain a
                  70-character sha256 reference, and a rule that only permits
                  breaking does not shrink the row's *minimum* width — which
                  propagates up the flex chain and stretches the whole dialog
                  past its own edge. Observed, then fixed. */}
              <span className="min-w-0 break-words">{option.label}</span>
              {option.badge === undefined ? null : (
                // Right-aligned in its own auto column, so at a narrow width the
                // name wraps and the badge stays put rather than the badge
                // squeezing the name it is describing.
                <Badge variant="secondary" className="justify-self-end">
                  {option.badge}
                </Badge>
              )}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
