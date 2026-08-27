import * as React from "react";

import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";

export type SearchablePickerOption = {
  id: string;
  label: string;
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
              className="h-auto justify-start whitespace-normal text-left"
              disabled={disabled}
              aria-pressed={option.id === selectedId}
              onClick={() => onSelect(option.id)}
            >
              {option.label}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}
