import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle, Settings2 } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Switch } from "@loom/ui-kit/components/ui/switch";
import {
  ActionParamsDialog,
  takesParameters,
} from "@loom/ui-kit/components/ActionParamsDialog";
import { dashboardsQueryKey } from "@loom/ui-kit/components/DashboardSidebar";
import {
  SearchablePickerList,
  type SearchablePickerOption,
} from "@loom/ui-kit/components/SearchablePickerList";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import type { ConnectorAction, PlacementAction, SubTarget } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { matchesTarget } from "@loom/ui-kit/lib/connector-details";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { describeTargetKind } from "@loom/ui-kit/lib/target-label";

type ActionKind = "navigate" | "connectorAction";

/** The host view, as a `Select` value — Radix reserves the empty string. */
const HOST_TARGET = "__host__";

/**
 * Configures what one placement does when it is clicked.
 *
 * ## Why this is its own component
 *
 * Three flows produce a `PlacementAction` and they must produce the same one:
 * the Button branch of `AddPlacementDialog` (where it is mandatory), the
 * connector branch of the same dialog, and `PlacementBindingsDialog` on a tile
 * that already exists (where it is optional and off by default). A second
 * implementation of the connector → target → action walk would drift from this
 * one, and the drift would be invisible until someone configured a tile that
 * the backend then refused.
 *
 * ## Required versus optional
 *
 * `required` forces the sub-form open and hides the toggle. It is set for a
 * static tile, because the backend refuses a placement that has neither a
 * connector nor an action — a tile with nothing to show and nothing to do is a
 * blank rectangle. Everywhere else the toggle starts off, and turning it off
 * again emits `null`, which is how a tile loses its click behaviour.
 *
 * ## Parameters are chosen now, not at click time
 *
 * A `connectorAction` tile stores its parameters, so clicking it is one click.
 * They are collected through the same `ActionParamsDialog` every other action
 * uses — the same schema, the same validation — rather than a second parameter
 * form that would drift from it. That dialog owns a `<form>`, which is also why
 * it stays a dialog here instead of being inlined into the form this editor
 * already sits inside.
 */
export function PlacementActionEditor({
  value,
  onChange,
  /** The dashboard being edited. Excluded from the navigate picker: a tile that
   *  navigates to the page it is already on does nothing a user can see. */
  currentDashboardId,
  required = false,
  disabled = false,
}: {
  value: PlacementAction | null;
  onChange: (next: PlacementAction | null) => void;
  currentDashboardId: string;
  required?: boolean;
  disabled?: boolean;
}) {
  const api = useApiClient();
  const enabled = required || value !== null;
  const kind: ActionKind = value?.type ?? "navigate";

  // Only ever fetched once the sub-form is open. A connector tile with no click
  // behaviour is the common case, and it should cost no requests.
  const dashboards = useQuery({
    queryKey: dashboardsQueryKey,
    queryFn: ({ signal }) => api.getDashboards(signal),
    enabled: enabled && kind === "navigate",
  });

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: enabled && kind === "connectorAction",
  });

  const instanceId =
    value?.type === "connectorAction" ? value.connectorInstanceId : null;

  const detail = useQuery({
    queryKey: ["connector-instance", instanceId],
    queryFn: ({ signal }) => api.getConnectorInstance(instanceId as string, signal),
    enabled: instanceId !== null,
  });

  const subTargets = useQuery({
    queryKey: ["connector-instance-sub-targets", instanceId],
    queryFn: ({ signal }) => api.getSubTargets(instanceId as string, signal),
    enabled: instanceId !== null && detail.data?.supportsSubTargets === true,
    staleTime: 30_000,
  });

  // Every action the chosen instance advertises for the chosen target — the
  // same `matchesTarget` filter the binding editor uses, plus the row and kind
  // actions of its resource kinds, which the backend accepts here too.
  const resourceKinds = useQuery({
    queryKey: [
      "connector-resource-kinds",
      instanceId,
      value?.type === "connectorAction" ? value.targetId : null,
    ],
    queryFn: ({ signal }) =>
      api.getResourceKinds(
        instanceId as string,
        value?.type === "connectorAction" ? value.targetId : null,
        signal,
      ),
    enabled: instanceId !== null,
    staleTime: 5 * 60_000,
  });

  const actionTargetId = value?.type === "connectorAction" ? value.targetId : null;
  const availableActions: ConnectorAction[] = React.useMemo(() => {
    const direct = (detail.data?.actions ?? []).filter((action) =>
      matchesTarget(action, actionTargetId),
    );
    // A resource action is an ordinary connector action advertised somewhere
    // else, and the click endpoint resolves it through the same lookup. Row
    // actions are deliberately absent: they need a `resourceId` naming a row
    // that does not exist until the table is drawn, which a tile cannot supply.
    const kinds = (resourceKinds.data ?? []).flatMap((entry) => entry.kindActions);
    const seen = new Set(direct.map((action) => action.id));
    return [...direct, ...kinds.filter((action) => !seen.has(action.id))];
  }, [actionTargetId, detail.data, resourceKinds.data]);

  const selectedAction =
    value?.type === "connectorAction"
      ? availableActions.find((action) => action.id === value.actionId)
      : undefined;
  const [paramsOpen, setParamsOpen] = React.useState(false);

  function setKind(next: ActionKind) {
    onChange(
      next === "navigate"
        ? { type: "navigate", targetDashboardId: "" }
        : {
            type: "connectorAction",
            connectorInstanceId: "",
            targetId: null,
            actionId: "",
            params: {},
          },
    );
  }

  function patchConnectorAction(
    patch: Partial<Extract<PlacementAction, { type: "connectorAction" }>>,
  ) {
    if (value?.type !== "connectorAction") return;
    onChange({ ...value, ...patch });
  }

  return (
    <div className="flex flex-col gap-4">
      {required ? null : (
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <Label htmlFor="placement-clickable" className="text-sm font-medium">
              Make this tile clickable
            </Label>
            <p className="text-xs text-muted-foreground">
              Clicking anywhere on the tile opens another dashboard or runs one
              connector action. Its widgets keep working as they do now.
            </p>
          </div>
          <Switch
            id="placement-clickable"
            checked={enabled}
            disabled={disabled}
            onCheckedChange={(checked) =>
              checked ? setKind("navigate") : onChange(null)
            }
          />
        </div>
      )}

      {enabled ? (
        <div className="flex flex-col gap-4 rounded-md border p-3">
          <div className="flex flex-col gap-2">
            <Label>When clicked</Label>
            <SegmentedControl
              label="Click behaviour"
              value={kind}
              options={[
                { value: "navigate", label: "Go to a dashboard" },
                { value: "connectorAction", label: "Run an action" },
              ]}
              onChange={setKind}
            />
          </div>

          {kind === "navigate" ? (
            <div className="flex min-h-0 flex-col gap-2">
              <Label>Dashboard to open</Label>
              {dashboards.isPending ? (
                <Skeleton className="h-24 w-full" />
              ) : dashboards.isError ? (
                <Alert variant="destructive">
                  <AlertCircle aria-hidden="true" />
                  <AlertDescription>
                    {describeConnectorError(dashboards.error)}
                  </AlertDescription>
                </Alert>
              ) : (
                <>
                  <SearchablePickerList
                    options={dashboards.data
                      .filter((dashboard) => dashboard.id !== currentDashboardId)
                      .map(
                        (dashboard): SearchablePickerOption => ({
                          id: dashboard.id,
                          label: dashboard.name,
                          // Hidden dashboards are exactly what a navigate tile
                          // is usually for, so they are offered here even
                          // though the sidebar leaves them out — and badged, so
                          // nobody wonders why they cannot find it afterwards.
                          badge: dashboard.hidden ? "Hidden" : undefined,
                        }),
                      )}
                    searchLabel="Search dashboards"
                    emptyMessage="No other dashboards you can open"
                    selectedId={
                      value?.type === "navigate" ? value.targetDashboardId : null
                    }
                    disabled={disabled}
                    onSelect={(next) =>
                      onChange({ type: "navigate", targetDashboardId: next })
                    }
                  />
                  <p className="text-xs text-muted-foreground">
                    Only dashboards you can open are listed. Everyone who clicks
                    this tile is checked separately, so someone without access
                    to that dashboard will be told so rather than taken there.
                  </p>
                </>
              )}
            </div>
          ) : (
            <div className="flex flex-col gap-4">
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="placement-action-instance">Connector</Label>
                {instances.isPending ? (
                  <Skeleton className="h-9 w-full" />
                ) : instances.isError ? (
                  <Alert variant="destructive">
                    <AlertCircle aria-hidden="true" />
                    <AlertDescription>
                      {describeConnectorError(instances.error)}
                    </AlertDescription>
                  </Alert>
                ) : instances.data.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    No connector instances exist yet. Add one under Connectors
                    first.
                  </p>
                ) : (
                  <Select
                    value={instanceId ?? ""}
                    disabled={disabled}
                    onValueChange={(next) =>
                      // Target and action belong to the connector that
                      // advertised them, so both are cleared rather than
                      // carried onto a connector that has never heard of them.
                      patchConnectorAction({
                        connectorInstanceId: next,
                        targetId: null,
                        actionId: "",
                        params: {},
                      })
                    }
                  >
                    <SelectTrigger id="placement-action-instance">
                      <SelectValue placeholder="Choose a connector" />
                    </SelectTrigger>
                    <SelectContent>
                      {instances.data.map((instance) => (
                        <SelectItem key={instance.id} value={instance.id}>
                          {instance.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              </div>

              {instanceId === null || instanceId === "" ? null : detail.isPending ? (
                <Skeleton className="h-24 w-full" />
              ) : detail.isError ? (
                <Alert variant="destructive">
                  <AlertCircle aria-hidden="true" />
                  <AlertDescription>
                    {describeConnectorError(detail.error)}
                  </AlertDescription>
                </Alert>
              ) : (
                <>
                  {detail.data.supportsSubTargets ? (
                    <div className="flex flex-col gap-1.5">
                      <Label htmlFor="placement-action-target">View</Label>
                      <Select
                        value={actionTargetId ?? HOST_TARGET}
                        disabled={disabled || subTargets.isPending}
                        onValueChange={(next) =>
                          patchConnectorAction({
                            targetId: next === HOST_TARGET ? null : next,
                            // Actions are declared per target, so the chosen
                            // one may not exist on the new view.
                            actionId: "",
                            params: {},
                          })
                        }
                      >
                        <SelectTrigger id="placement-action-target">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={HOST_TARGET}>Server info</SelectItem>
                          {(subTargets.data ?? []).map((target: SubTarget) => (
                            <SelectItem key={target.id} value={target.id}>
                              {target.label}
                              {describeTargetKind(target.kind) === null
                                ? ""
                                : ` · ${describeTargetKind(target.kind)}`}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  ) : null}

                  <div className="flex flex-col gap-1.5">
                    <Label htmlFor="placement-action-id">Action</Label>
                    {availableActions.length === 0 ? (
                      <p className="text-sm text-muted-foreground">
                        This connector advertises no actions for that view.
                      </p>
                    ) : (
                      <Select
                        value={
                          value?.type === "connectorAction" ? value.actionId : ""
                        }
                        disabled={disabled}
                        onValueChange={(next) =>
                          // Parameters belong to the action that declared them.
                          patchConnectorAction({ actionId: next, params: {} })
                        }
                      >
                        <SelectTrigger id="placement-action-id">
                          <SelectValue placeholder="Choose an action" />
                        </SelectTrigger>
                        <SelectContent>
                          {availableActions.map((action) => (
                            <SelectItem key={action.id} value={action.id}>
                              {action.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    )}
                  </div>

                  {selectedAction !== undefined && takesParameters(selectedAction) ? (
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={disabled}
                        onClick={() => setParamsOpen(true)}
                      >
                        <Settings2 data-icon="inline-start" aria-hidden="true" />
                        Set parameters
                      </Button>
                      <Badge variant="secondary" className="font-mono text-[11px]">
                        {describeParams(
                          value?.type === "connectorAction" ? value.params : {},
                        )}
                      </Badge>
                      <p className="w-full text-xs text-muted-foreground">
                        Chosen now and stored on the tile, so clicking it later
                        is one click rather than a form.
                      </p>
                    </div>
                  ) : null}

                  {selectedAction?.isDisruptive === true ? (
                    <p className="text-xs text-muted-foreground">
                      This action interrupts the service. Anyone clicking the
                      tile still needs permission to control this connector.
                    </p>
                  ) : null}
                </>
              )}
            </div>
          )}
        </div>
      ) : null}

      {paramsOpen && selectedAction !== undefined ? (
        <ActionParamsDialog
          action={selectedAction}
          connectorName={detail.data?.name ?? "this connector"}
          isPending={false}
          submitLabel="Save parameters"
          onOpenChange={(open) => setParamsOpen(open)}
          onSubmit={(params) => {
            patchConnectorAction({ params });
            setParamsOpen(false);
          }}
        />
      ) : null}
    </div>
  );
}

/**
 * Whether a `PlacementAction` is complete enough to send.
 *
 * Exported so the dialogs that host this editor can disable their submit button
 * instead of letting the backend answer 400 for a half-filled sub-form. It is a
 * completeness check and not a validity one: whether the target exists, and
 * whether the caller may reach it, are the backend's answers to give.
 */
export function isPlacementActionComplete(action: PlacementAction | null): boolean {
  if (action === null) return false;
  return action.type === "navigate"
    ? action.targetDashboardId !== ""
    : action.connectorInstanceId !== "" && action.actionId !== "";
}

/** A short, honest summary of stored parameters for the badge beside the button. */
function describeParams(params: unknown): string {
  if (typeof params !== "object" || params === null || Array.isArray(params)) {
    return "No parameters set";
  }
  const entries = Object.entries(params as Record<string, unknown>);
  if (entries.length === 0) return "No parameters set";
  return entries.map(([key, entry]) => `${key}=${String(entry)}`).join(", ");
}
