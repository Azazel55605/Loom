import * as React from "react";
import { AlertCircle, Clipboard, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { CapabilityStatusList } from "@loom/ui-kit/components/CapabilityStatusList";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import type {
  ConnectionTestResult,
  SetupGuideVariant,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import {
  computeCapabilitySummary,
  renderSetupGuideTemplate,
} from "@loom/ui-kit/lib/setup-guide";
import { cn } from "@loom/ui-kit/lib/utils";

type ConnectionTestState =
  | { state: "idle" }
  | { state: "pending" }
  | { state: "success"; result: ConnectionTestResult }
  | { state: "error"; message: string };

function initialToggleValues(variant: SetupGuideVariant): Record<string, boolean> {
  return Object.fromEntries(variant.toggles.map((toggle) => [toggle.key, toggle.default]));
}

/** One setup path: local toggles, rendered instructions, and an independent live test. */
export function SetupGuideVariantPanel({
  variant,
  typeId,
  formValues,
}: {
  variant: SetupGuideVariant;
  typeId: string;
  formValues: Record<string, unknown>;
}) {
  const api = useApiClient();
  const switchPrefix = React.useId();
  const [toggleValues, setToggleValues] = React.useState<Record<string, boolean>>(() =>
    initialToggleValues(variant),
  );
  const [connectionTest, setConnectionTest] = React.useState<ConnectionTestState>({
    state: "idle",
  });
  const request = React.useRef<AbortController | null>(null);

  React.useEffect(() => {
    request.current?.abort();
    setConnectionTest({ state: "idle" });
    return () => request.current?.abort();
  }, [typeId, formValues]);

  const declarative = computeCapabilitySummary(toggleValues, variant.capabilityRequirements);
  const templateValues = {
    ...formValues,
    ...Object.fromEntries(
      variant.toggles.map((toggle) => [toggle.envVar, toggleValues[toggle.key] ? "1" : "0"]),
    ),
  };
  const rendered = renderSetupGuideTemplate(variant.template, templateValues);

  async function copy() {
    try {
      await navigator.clipboard.writeText(rendered);
      toast.success("Setup guide copied.");
    } catch {
      toast.error("Could not copy the setup guide.");
    }
  }

  async function testConnection() {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setConnectionTest({ state: "pending" });

    try {
      const result = await api.testConnectionForType(typeId, formValues, controller.signal);
      if (!controller.signal.aborted) setConnectionTest({ state: "success", result });
    } catch (error: unknown) {
      if (!controller.signal.aborted) {
        setConnectionTest({ state: "error", message: describeAdminFailure(error).message });
      }
    } finally {
      if (request.current === controller) request.current = null;
    }
  }

  return (
    <div className="flex flex-col gap-5">
      <p className="text-sm text-muted-foreground">{variant.description}</p>

      {variant.toggles.length > 0 ? (
        <section className="flex flex-col gap-3" aria-label={`${variant.label} options`}>
          <div className="flex flex-col gap-2">
            {variant.toggles.map((toggle) => {
              const id = `${switchPrefix}-${toggle.key}`;
              return (
                <div key={toggle.key} className="flex min-h-11 items-center gap-3 rounded-md border p-3">
                  <Switch
                    id={id}
                    checked={toggleValues[toggle.key] ?? false}
                    onCheckedChange={(checked) =>
                      setToggleValues((current) => ({ ...current, [toggle.key]: checked }))
                    }
                  />
                  <Label htmlFor={id} className="flex min-w-0 flex-1 cursor-pointer flex-col gap-1">
                    <span className="flex flex-wrap items-center gap-2">
                      <span>{toggle.label}</span>
                      {toggle.recommended ? <Badge variant="secondary">Recommended</Badge> : null}
                    </span>
                    <span className="text-xs font-normal text-muted-foreground">
                      {toggle.description}
                    </span>
                  </Label>
                </div>
              );
            })}
          </div>

          {declarative.summarySentence !== "" ? (
            <p className="text-sm" aria-live="polite">
              {declarative.summarySentence}
            </p>
          ) : null}
          <CapabilityStatusList capabilities={declarative.capabilities} />
        </section>
      ) : null}

      <section className="flex flex-col gap-3" aria-label="Rendered setup instructions">
        <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-muted/40 p-3 text-xs">
          <code>{rendered}</code>
        </pre>
        <div className="flex justify-end">
          <Button type="button" variant="outline" size="sm" onClick={() => void copy()}>
            <Clipboard data-icon="inline-start" aria-hidden="true" />
            Copy
          </Button>
        </div>
      </section>

      <section className="flex flex-col gap-3 border-t border-border pt-4" aria-label="Live connection test">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium">Live capability check</p>
            <p className="text-xs text-muted-foreground">
              Tests the current connector configuration without saving it.
            </p>
          </div>
          <Button
            type="button"
            size="sm"
            disabled={connectionTest.state === "pending"}
            onClick={() => void testConnection()}
          >
            {connectionTest.state === "pending" ? (
              <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
            ) : null}
            {connectionTest.state === "pending" ? "Testing…" : "Test connection"}
          </Button>
        </div>

        {connectionTest.state === "error" ? (
          <Alert variant="destructive">
            <AlertCircle aria-hidden="true" />
            <AlertTitle>Connection test failed</AlertTitle>
            <AlertDescription>{connectionTest.message}</AlertDescription>
          </Alert>
        ) : null}

        {connectionTest.state === "success" ? (
          <div className="flex flex-col gap-3" aria-live="polite">
            <div className="flex items-start gap-2 text-sm">
              <span
                className={cn(
                  "mt-1.5 size-2 shrink-0 rounded-full",
                  connectionTest.result.reachable ? "bg-status-healthy" : "bg-status-down",
                )}
                aria-hidden="true"
              />
              <span>
                <span className="font-medium">
                  {connectionTest.result.reachable ? "Connection reachable" : "Connection unavailable"}
                </span>
                {connectionTest.result.message !== null ? (
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    {connectionTest.result.message}
                  </span>
                ) : null}
              </span>
            </div>
            {connectionTest.result.capabilities.length > 0 ? (
              <CapabilityStatusList capabilities={connectionTest.result.capabilities} />
            ) : (
              <p className="text-xs text-muted-foreground">
                This connector reports reachability without capability-level detail.
              </p>
            )}
          </div>
        ) : null}
      </section>
    </div>
  );
}
