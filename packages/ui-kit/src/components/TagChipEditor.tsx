import * as React from "react";
import { Plus, X } from "lucide-react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";

/** Free-form tag entry with removable chips and in-use tag suggestions. */
export function TagChipEditor({
  value,
  suggestions,
  onChange,
  disabled = false,
}: {
  value: string[];
  suggestions: string[];
  onChange: (tags: string[]) => void;
  disabled?: boolean;
}) {
  const [draft, setDraft] = React.useState("");
  const deferredDraft = React.useDeferredValue(draft);

  const matches = React.useMemo(() => {
    const needle = deferredDraft.trim().toLocaleLowerCase();
    if (needle === "") return [];
    const selected = new Set(value);
    return suggestions
      .filter(
        (suggestion) =>
          !selected.has(suggestion) && suggestion.toLocaleLowerCase().includes(needle),
      )
      .slice(0, 6);
  }, [deferredDraft, suggestions, value]);

  function addTag(candidate: string) {
    const tag = candidate.trim();
    if (tag === "") return;
    if (!value.includes(tag)) onChange([...value, tag]);
    setDraft("");
  }

  function removeTag(tag: string) {
    onChange(value.filter((candidate) => candidate !== tag));
  }

  return (
    <div className="flex flex-col gap-2">
      {value.length > 0 ? (
        <div className="flex flex-wrap gap-1" aria-label="Assigned tags">
          {value.map((tag) => (
            <Badge key={tag} variant="secondary" className="gap-1 pr-1">
              {tag}
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="size-5 rounded-full"
                disabled={disabled}
                aria-label={`Remove ${tag}`}
                onClick={() => removeTag(tag)}
              >
                <X aria-hidden="true" />
              </Button>
            </Badge>
          ))}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">No tags assigned.</p>
      )}

      <div className="flex gap-2">
        <Input
          value={draft}
          disabled={disabled}
          placeholder="Add a tag"
          aria-label="New connector tag"
          onChange={(event) => {
            const next = event.target.value;
            if (!next.includes(",")) {
              setDraft(next);
              return;
            }

            const pieces = next.split(",");
            const trailing = pieces.pop() ?? "";
            const additions = pieces.map((piece) => piece.trim()).filter(Boolean);
            if (additions.length > 0) {
              onChange(Array.from(new Set([...value, ...additions])));
            }
            setDraft(trailing);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === ",") {
              event.preventDefault();
              addTag(draft);
            } else if (event.key === "Backspace" && draft === "" && value.length > 0) {
              removeTag(value[value.length - 1]);
            }
          }}
        />
        <Button
          type="button"
          variant="outline"
          size="icon"
          disabled={disabled || draft.trim() === ""}
          aria-label="Add tag"
          onClick={() => addTag(draft)}
        >
          <Plus aria-hidden="true" />
        </Button>
      </div>

      {matches.length > 0 ? (
        <div className="surface-panel flex flex-col gap-1 rounded-md border p-1" aria-label="Tag suggestions">
          {matches.map((suggestion) => (
            <Button
              key={suggestion}
              type="button"
              variant="ghost"
              size="sm"
              className="justify-start"
              disabled={disabled}
              onClick={() => addTag(suggestion)}
            >
              {suggestion}
            </Button>
          ))}
        </div>
      ) : null}

      <p className="text-xs text-muted-foreground">
        Press Enter or comma to add a tag. Existing tags are suggested, but new ones are allowed.
      </p>
    </div>
  );
}
