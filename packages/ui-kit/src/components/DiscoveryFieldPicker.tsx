import * as React from "react";
import { AlertCircle, Search } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@loom/ui-kit/components/ui/popover";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import type { DiscoveredResource } from "@loom/ui-kit/lib/api";

/**
 * A normal text field with optional discovery assistance.
 *
 * Manual entry remains the primary control. Discovery only supplies possible
 * values and knows nothing about connector types, candidate configuration, or
 * where the selected value will eventually be stored.
 */
export function DiscoveryFieldPicker({
  fieldName,
  currentValue,
  onSelect,
  canDiscover,
  onDiscover,
  disabled = false,
  inputId,
  ariaInvalid,
  ariaDescribedBy,
}: {
  fieldName: string;
  currentValue: unknown;
  onSelect: (value: unknown) => void;
  canDiscover: boolean;
  onDiscover: () => Promise<DiscoveredResource[]>;
  disabled?: boolean;
  inputId?: string;
  ariaInvalid?: boolean;
  ariaDescribedBy?: string;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [resources, setResources] = React.useState<DiscoveredResource[]>([]);
  const [failure, setFailure] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const requestId = React.useRef(0);

  const filteredResources = React.useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (needle === "") return resources;
    return resources.filter((resource) =>
      resource.suggestedName.toLocaleLowerCase().includes(needle),
    );
  }, [query, resources]);

  async function loadResources() {
    const currentRequest = requestId.current + 1;
    requestId.current = currentRequest;
    setIsLoading(true);
    setFailure(null);
    setResources([]);

    try {
      const discovered = await onDiscover();
      if (requestId.current === currentRequest) setResources(discovered);
    } catch (error: unknown) {
      if (requestId.current === currentRequest) {
        setFailure(describeAdminFailure(error).message);
      }
    } finally {
      if (requestId.current === currentRequest) setIsLoading(false);
    }
  }

  function changeOpen(next: boolean) {
    setOpen(next);
    if (!next) {
      requestId.current += 1;
      return;
    }
    setQuery("");
    void loadResources();
  }

  return (
    <Popover open={open} onOpenChange={changeOpen}>
      <div className="flex items-center gap-2">
        <Input
          id={inputId}
          disabled={disabled}
          aria-invalid={ariaInvalid}
          aria-describedby={ariaDescribedBy}
          value={currentValue === undefined || currentValue === null ? "" : String(currentValue)}
          onChange={(event) => {
            const value = event.target.value;
            onSelect(value === "" ? undefined : value);
          }}
        />
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            size="icon"
            disabled={disabled || !canDiscover}
            aria-label={`Browse ${fieldName}`}
          >
            <Search aria-hidden="true" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent
        align="end"
        className="flex max-h-[min(24rem,var(--radix-popover-content-available-height))] w-[min(24rem,var(--radix-popover-content-available-width))] flex-col gap-3"
      >
        <Input
          aria-label={`Search ${fieldName}`}
          placeholder="Search options"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />

        {failure !== null ? (
          <Alert variant="destructive">
            <AlertCircle aria-hidden="true" />
            <AlertTitle>Could not discover options</AlertTitle>
            <AlertDescription>{failure}</AlertDescription>
          </Alert>
        ) : isLoading ? (
          <div className="flex flex-col gap-2" aria-label="Discovering options">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-3/4" />
          </div>
        ) : filteredResources.length === 0 ? (
          <p className="py-3 text-center text-sm text-muted-foreground">No options found</p>
        ) : (
          <div className="flex min-h-0 flex-col gap-1 overflow-y-auto">
            {filteredResources.map((resource, index) => (
              <Button
                key={`${resource.suggestedName}:${index}`}
                type="button"
                variant="ghost"
                className="h-auto justify-start whitespace-normal text-left"
                onClick={() => {
                  onSelect(resource.targetFieldValue);
                  setOpen(false);
                }}
              >
                {resource.suggestedName}
              </Button>
            ))}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
