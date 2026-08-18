import * as React from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/**
 * Lets the user point the desktop app at their own Loom backend.
 *
 * The web frontend bakes its API URL in at build time, which works because it
 * is served by the same deployment it talks to. A desktop build is downloaded
 * once and run against whatever server the user happens to have, so the URL has
 * to be editable at runtime and persisted between launches.
 *
 * A thin composition of shadcn `Input` + `Button`, not a new primitive: it adds
 * the submit-on-enter form wrapper and the labelling, and nothing else.
 */
export function ServerUrlField({
  value,
  onSubmit,
  disabled,
}: {
  value: string;
  onSubmit: (url: string) => void;
  disabled?: boolean;
}) {
  const [draft, setDraft] = React.useState(value);

  // Keep the draft in step when the URL changes elsewhere (e.g. restored from
  // storage after the first render).
  React.useEffect(() => setDraft(value), [value]);

  return (
    <form
      className="flex items-end gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        const trimmed = draft.trim();
        if (trimmed) onSubmit(trimmed);
      }}
    >
      <div className="flex-1 space-y-1.5">
        <label
          htmlFor="server-url"
          className="text-xs font-medium text-muted-foreground"
        >
          Server URL
        </label>
        <Input
          id="server-url"
          name="server-url"
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          placeholder="http://localhost:8080"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
      </div>
      <Button type="submit" disabled={disabled || !draft.trim()}>
        Connect
      </Button>
    </form>
  );
}
