import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertCircle, Settings2 } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
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
import { DataPointPicker } from "@loom/ui-kit/components/DataPointPicker";
import { IconPickerPopover } from "@loom/ui-kit/components/IconPicker";
import {
  SearchablePickerList,
  type SearchablePickerOption,
} from "@loom/ui-kit/components/SearchablePickerList";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import type {
  ConnectorAction,
  DataPointDescriptor,
  PlacementAction,
  PlacementActionTransition,
  StateButtonColor,
  StateButtonDisplay,
  SubTarget,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { matchesTarget } from "@loom/ui-kit/lib/connector-details";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { describeTargetKind } from "@loom/ui-kit/lib/target-label";

type ActionKind = "navigate" | "connectorAction";
export type PlacementActionEditorSection = "all" | "click" | "state" | "display";

/** The host view, as a `Select` value — Radix reserves the empty string. */
const HOST_TARGET = "__host__";
const STATE_BUTTON_COLORS: { value: StateButtonColor; label: string }[] = [
  { value: "success", label: "Success" },
  { value: "warning", label: "Warning" },
  { value: "error", label: "Error" },
  { value: "neutral", label: "Neutral" },
];
const BOOLEAN_DATA_POINT_TYPES = ["bool"] as const;

type ParamsPurpose = "base" | "toTrue" | "toFalse";

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
  section = "all",
}: {
  value: PlacementAction | null;
  onChange: (next: PlacementAction | null) => void;
  currentDashboardId: string;
  required?: boolean;
  disabled?: boolean;
  /** A wizard host can expose one focused part without duplicating the editor. */
  section?: PlacementActionEditorSection;
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
  const [paramsPurpose, setParamsPurpose] = React.useState<ParamsPurpose | null>(null);
  const connectorAction = value?.type === "connectorAction" ? value : null;
  const stateAware = connectorAction?.stateDataPointId != null;
  const [separateTransitionActions, setSeparateTransitionActions] = React.useState(
    connectorAction?.toTrue != null &&
      connectorAction.toFalse != null &&
      connectorAction.toTrue.actionId !== connectorAction.toFalse.actionId,
  );

  React.useEffect(() => {
    if (connectorAction?.toTrue == null || connectorAction.toFalse == null) return;
    setSeparateTransitionActions(
      connectorAction.toTrue.actionId !== connectorAction.toFalse.actionId,
    );
  }, [connectorAction?.toFalse?.actionId, connectorAction?.toTrue?.actionId]);

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

  function clearStateAwareness() {
    setSeparateTransitionActions(false);
    patchConnectorAction({
      stateDataPointId: null,
      stateTargetId: null,
      renderStyle: null,
      toTrue: null,
      toFalse: null,
      stateButtonDisplay: null,
    });
  }

  function enableStateAwareness() {
    const transition =
      selectedAction === undefined
        ? null
        : { actionId: selectedAction.id, params: {} };
    patchConnectorAction({
      // Empty keeps the section enabled while correctly failing the local
      // completeness check until a Boolean point is chosen.
      stateDataPointId: "",
      stateTargetId: actionTargetId,
      renderStyle: "switch",
      toTrue: transition,
      toFalse: transition === null ? null : { ...transition },
      stateButtonDisplay: null,
    });
  }

  function patchTransition(
    direction: "toTrue" | "toFalse",
    transition: PlacementActionTransition | null,
  ) {
    patchConnectorAction({ [direction]: transition });
  }

  const paramsAction =
    paramsPurpose === "base"
      ? selectedAction
      : availableActions.find(
          (action) => action.id === connectorAction?.[paramsPurpose ?? "toTrue"]?.actionId,
        );

  function handleSeparateActionsChange(separate: boolean) {
    setSeparateTransitionActions(separate);
    if (separate || connectorAction === null) return;
    const actionId =
      connectorAction.toTrue?.actionId ??
      connectorAction.toFalse?.actionId ??
      connectorAction.actionId;
    if (actionId === "") return;
    patchConnectorAction({
      toTrue: {
        actionId,
        params:
          connectorAction.toTrue?.actionId === actionId
            ? connectorAction.toTrue.params
            : {},
      },
      toFalse: {
        actionId,
        params:
          connectorAction.toFalse?.actionId === actionId
            ? connectorAction.toFalse.params
            : {},
      },
    });
  }

  function renderStateConfiguration(mode: "all" | "state" | "display") {
    if (connectorAction === null) {
      return (
        <p className="text-sm text-muted-foreground">
          Dashboard navigation does not use connector state.
        </p>
      );
    }
    if (detail.isPending) return <Skeleton className="h-24 w-full" />;
    if (detail.isError) {
      return (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertDescription>{describeConnectorError(detail.error)}</AlertDescription>
        </Alert>
      );
    }
    if (detail.data === undefined) return null;

    return (
      <>
        {mode !== "display" ? (
          <div className="flex items-start justify-between gap-4 rounded-md border p-3">
            <div className="min-w-0">
              <Label
                htmlFor="placement-action-state-aware"
                className="text-sm font-medium"
              >
                Make this state-aware
              </Label>
              <p className="text-xs text-muted-foreground">
                Read a live on/off value and let the server choose the correct
                transition when this control is pressed.
              </p>
            </div>
            <Switch
              id="placement-action-state-aware"
              checked={stateAware}
              disabled={disabled}
              onCheckedChange={(checked) =>
                checked ? enableStateAwareness() : clearStateAwareness()
              }
            />
          </div>
        ) : null}

        {stateAware ? (
          <StateAwareActionEditor
            action={connectorAction}
            actions={availableActions}
            dataPoints={detail.data.dataPoints}
            disabled={disabled}
            separateActions={separateTransitionActions}
            section={mode}
            onSeparateActionsChange={handleSeparateActionsChange}
            onPatch={patchConnectorAction}
            onPatchTransition={patchTransition}
            onEditParams={setParamsPurpose}
          />
        ) : mode === "state" ? (
          <p className="text-sm text-muted-foreground">
            This button will run the selected action without reading live state.
          </p>
        ) : null}
      </>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {required || section !== "all" ? null : (
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

      {enabled && (section === "all" || section === "click") ? (
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
                        stateDataPointId: null,
                        stateTargetId: null,
                        renderStyle: null,
                        toTrue: null,
                        toFalse: null,
                        stateButtonDisplay: null,
                      })
                    }
                  >
                    <SelectTrigger id="placement-action-instance">
                      <SelectValue placeholder="Choose a connector" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        {instances.data.map((instance) => (
                          <SelectItem key={instance.id} value={instance.id}>
                            {instance.name}
                          </SelectItem>
                        ))}
                      </SelectGroup>
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
                            stateDataPointId: null,
                            stateTargetId: null,
                            renderStyle: null,
                            toTrue: null,
                            toFalse: null,
                            stateButtonDisplay: null,
                          })
                        }
                      >
                        <SelectTrigger id="placement-action-target">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={HOST_TARGET}>Server info</SelectItem>
                            {(subTargets.data ?? []).map((target: SubTarget) => (
                              <SelectItem key={target.id} value={target.id}>
                                {target.label}
                                {describeTargetKind(target.kind) === null
                                  ? ""
                                  : ` · ${describeTargetKind(target.kind)}`}
                              </SelectItem>
                            ))}
                          </SelectGroup>
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
                          <SelectGroup>
                            {availableActions.map((action) => (
                              <SelectItem key={action.id} value={action.id}>
                                {action.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
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
                        onClick={() => setParamsPurpose("base")}
                      >
                        <Settings2 data-icon="inline-start" aria-hidden="true" />
                        Set parameters
                      </Button>
                      <Badge variant="secondary" className="font-mono text-[0.6875rem]">
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

                  {section === "all" ? renderStateConfiguration("all") : null}
                </>
              )}
            </div>
          )}
        </div>
      ) : null}

      {enabled && section === "state" ? renderStateConfiguration("state") : null}
      {enabled && section === "display" ? renderStateConfiguration("display") : null}

      {paramsPurpose !== null && paramsAction !== undefined ? (
        <ActionParamsDialog
          action={paramsAction}
          connectorName={detail.data?.name ?? "this connector"}
          isPending={false}
          submitLabel="Save parameters"
          onOpenChange={(open) => !open && setParamsPurpose(null)}
          onSubmit={(params) => {
            if (paramsPurpose === "base") {
              patchConnectorAction({ params });
            } else {
              const transition = connectorAction?.[paramsPurpose];
              if (transition !== null && transition !== undefined) {
                patchTransition(paramsPurpose, { ...transition, params });
              }
            }
            setParamsPurpose(null);
          }}
        />
      ) : null}
    </div>
  );
}

function StateAwareActionEditor({
  action,
  actions,
  dataPoints,
  disabled,
  separateActions,
  section,
  onSeparateActionsChange,
  onPatch,
  onPatchTransition,
  onEditParams,
}: {
  action: Extract<PlacementAction, { type: "connectorAction" }>;
  actions: ConnectorAction[];
  dataPoints: DataPointDescriptor[];
  disabled: boolean;
  separateActions: boolean;
  section: "all" | "state" | "display";
  onSeparateActionsChange: (separate: boolean) => void;
  onPatch: (
    patch: Partial<Extract<PlacementAction, { type: "connectorAction" }>>,
  ) => void;
  onPatchTransition: (
    direction: "toTrue" | "toFalse",
    transition: PlacementActionTransition | null,
  ) => void;
  onEditParams: (purpose: "toTrue" | "toFalse") => void;
}) {
  const sharedActionId = action.toTrue?.actionId ?? action.toFalse?.actionId ?? "";
  const renderStyle = action.renderStyle ?? "switch";

  function setSharedAction(actionId: string) {
    onPatch({
      toTrue: {
        actionId,
        params: action.toTrue?.actionId === actionId ? action.toTrue.params : {},
      },
      toFalse: {
        actionId,
        params: action.toFalse?.actionId === actionId ? action.toFalse.params : {},
      },
    });
  }

  function setRenderStyle(style: "switch" | "stateButton") {
    onPatch({
      renderStyle: style,
      stateButtonDisplay:
        style === "stateButton"
          ? (action.stateButtonDisplay ?? defaultStateButtonDisplay())
          : null,
    });
  }

  return (
    <div className="surface-panel flex flex-col gap-4 rounded-md border p-3">
      {section !== "display" ? (
        <>
          <div className="flex flex-col gap-2">
            <Label>State data point</Label>
            <DataPointPicker
              idPrefix="placement-action-state"
              value={
                action.stateDataPointId === "" || action.stateDataPointId == null
                  ? null
                  : {
                      connectorInstanceId: action.connectorInstanceId,
                      targetId: action.stateTargetId ?? null,
                      dataPointId: action.stateDataPointId,
                    }
              }
              fixedContext={{
                connectorInstanceId: action.connectorInstanceId,
                targetId: action.targetId,
                dataPoints,
              }}
              allowedValueTypes={BOOLEAN_DATA_POINT_TYPES}
              disabled={disabled}
              onChange={(selection) =>
                onPatch({
                  stateDataPointId: selection?.dataPointId ?? "",
                  stateTargetId: selection?.targetId ?? action.targetId,
                })
              }
            />
            <p className="text-xs text-muted-foreground">
              Only Boolean readings on this connector view are available.
            </p>
          </div>

          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <Label htmlFor="placement-transition-actions">Use the same action</Label>
              <p className="text-xs text-muted-foreground">
                Keep this on when one action accepts different on/off parameters.
                Turn it off for separate actions such as Start and Stop.
              </p>
            </div>
            <Switch
              id="placement-transition-actions"
              checked={!separateActions}
              disabled={disabled}
              onCheckedChange={(same) => onSeparateActionsChange(!same)}
            />
          </div>

          {separateActions ? (
            <div className="grid gap-3 sm:grid-cols-2">
              <TransitionActionField
                title="When turning on"
                direction="toTrue"
                transition={action.toTrue ?? null}
                actions={actions}
                disabled={disabled}
                onChange={onPatchTransition}
                onEditParams={onEditParams}
              />
              <TransitionActionField
                title="When turning off"
                direction="toFalse"
                transition={action.toFalse ?? null}
                actions={actions}
                disabled={disabled}
                onChange={onPatchTransition}
                onEditParams={onEditParams}
              />
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              <ActionSelect
                id="placement-shared-transition-action"
                label="Transition action"
                value={sharedActionId}
                actions={actions}
                disabled={disabled}
                onChange={setSharedAction}
              />
              {sharedActionId === "" ? null : (
                <div className="grid gap-3 sm:grid-cols-2">
                  <TransitionParameters
                    title="When turning on"
                    direction="toTrue"
                    transition={action.toTrue ?? null}
                    actions={actions}
                    disabled={disabled}
                    onEditParams={onEditParams}
                  />
                  <TransitionParameters
                    title="When turning off"
                    direction="toFalse"
                    transition={action.toFalse ?? null}
                    actions={actions}
                    disabled={disabled}
                    onEditParams={onEditParams}
                  />
                </div>
              )}
            </div>
          )}

          <div className="flex flex-col gap-2">
            <Label>Render style</Label>
            <SegmentedControl
              label="State-aware action style"
              value={renderStyle}
              options={[
                { value: "switch", label: "Switch" },
                { value: "stateButton", label: "State Button" },
              ]}
              onChange={setRenderStyle}
            />
          </div>
        </>
      ) : null}

      {section !== "state" && renderStyle === "stateButton" && action.stateButtonDisplay != null ? (
        <div className="grid gap-4 sm:grid-cols-2">
          <StateButtonDisplayEditor
            title="When true"
            value={action.stateButtonDisplay.whenTrue}
            disabled={disabled}
            onChange={(whenTrue) =>
              onPatch({
                stateButtonDisplay: {
                  ...(action.stateButtonDisplay as StateButtonDisplay),
                  whenTrue,
                },
              })
            }
          />
          <StateButtonDisplayEditor
            title="When false"
            value={action.stateButtonDisplay.whenFalse}
            disabled={disabled}
            onChange={(whenFalse) =>
              onPatch({
                stateButtonDisplay: {
                  ...(action.stateButtonDisplay as StateButtonDisplay),
                  whenFalse,
                },
              })
            }
          />
        </div>
      ) : null}
    </div>
  );
}

function ActionSelect({
  id,
  label,
  value,
  actions,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  actions: ConnectorAction[];
  disabled: boolean;
  onChange: (actionId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Select value={value} disabled={disabled} onValueChange={onChange}>
        <SelectTrigger id={id}>
          <SelectValue placeholder="Choose an action" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            {actions.map((candidate) => (
              <SelectItem key={candidate.id} value={candidate.id}>
                {candidate.label}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </div>
  );
}

function TransitionActionField({
  title,
  direction,
  transition,
  actions,
  disabled,
  onChange,
  onEditParams,
}: {
  title: string;
  direction: "toTrue" | "toFalse";
  transition: PlacementActionTransition | null;
  actions: ConnectorAction[];
  disabled: boolean;
  onChange: (
    direction: "toTrue" | "toFalse",
    transition: PlacementActionTransition | null,
  ) => void;
  onEditParams: (purpose: "toTrue" | "toFalse") => void;
}) {
  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <ActionSelect
        id={`placement-${direction}-action`}
        label={title}
        value={transition?.actionId ?? ""}
        actions={actions}
        disabled={disabled}
        onChange={(actionId) => onChange(direction, { actionId, params: {} })}
      />
      <TransitionParameters
        title="Parameters"
        direction={direction}
        transition={transition}
        actions={actions}
        disabled={disabled}
        onEditParams={onEditParams}
      />
    </div>
  );
}

function TransitionParameters({
  title,
  direction,
  transition,
  actions,
  disabled,
  onEditParams,
}: {
  title: string;
  direction: "toTrue" | "toFalse";
  transition: PlacementActionTransition | null;
  actions: ConnectorAction[];
  disabled: boolean;
  onEditParams: (purpose: "toTrue" | "toFalse") => void;
}) {
  const selected = actions.find((candidate) => candidate.id === transition?.actionId);
  return (
    <div className="flex flex-col gap-2 rounded-md bg-muted/40 p-2">
      <p className="text-xs font-medium">{title}</p>
      {selected !== undefined && takesParameters(selected) ? (
        <>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onEditParams(direction)}
          >
            <Settings2 data-icon="inline-start" aria-hidden="true" />
            Set parameters
          </Button>
          <Badge variant="secondary" className="font-mono text-[0.6875rem]">
            {describeParams(transition?.params ?? {})}
          </Badge>
        </>
      ) : (
        <p className="text-xs text-muted-foreground">
          {selected === undefined ? "Choose an action first." : "No parameters needed."}
        </p>
      )}
    </div>
  );
}

function StateButtonDisplayEditor({
  title,
  value,
  disabled,
  onChange,
}: {
  title: string;
  value: StateButtonDisplay["whenTrue"];
  disabled: boolean;
  onChange: (value: StateButtonDisplay["whenTrue"]) => void;
}) {
  const id = React.useId();
  return (
    <div className="flex flex-col gap-3 rounded-md border p-3">
      <p className="text-sm font-medium">{title}</p>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${id}-label`}>Label</Label>
        <Input
          id={`${id}-label`}
          value={value.label}
          disabled={disabled}
          onChange={(event) => onChange({ ...value, label: event.target.value })}
        />
      </div>
      <IconPickerPopover
        label={`${title} icon`}
        value={value.icon === "" ? null : value.icon}
        defaultIcon="lucide:power"
        defaultLabel="Power"
        disabled={disabled}
        onChange={(icon) => onChange({ ...value, icon: icon ?? "lucide:power" })}
      />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${id}-color`}>Color</Label>
        <Select
          value={value.color ?? "neutral"}
          disabled={disabled}
          onValueChange={(color) =>
            onChange({ ...value, color: color as StateButtonColor })
          }
        >
          <SelectTrigger id={`${id}-color`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {STATE_BUTTON_COLORS.map((color) => (
                <SelectItem key={color.value} value={color.value}>
                  {color.label}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

function defaultStateButtonDisplay(): StateButtonDisplay {
  return {
    whenTrue: {
      label: "Turn off",
      icon: "lucide:power",
      color: "error",
    },
    whenFalse: {
      label: "Turn on",
      icon: "lucide:power",
      color: "success",
    },
  };
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
  if (!isPlacementActionClickComplete(action)) return false;
  if (action?.type !== "connectorAction") return true;
  if (!isPlacementActionStateComplete(action)) return false;
  if (!placementActionNeedsDisplayStep(action)) return true;
  const display = action.stateButtonDisplay;
  return (
    display != null &&
    display.whenTrue.label.trim() !== "" &&
    display.whenTrue.icon !== "" &&
    display.whenFalse.label.trim() !== "" &&
    display.whenFalse.icon !== ""
  );
}

/** The fields owned by the wizard's click-behaviour step. */
export function isPlacementActionClickComplete(action: PlacementAction | null): boolean {
  if (action === null) return false;
  if (action.type === "navigate") return action.targetDashboardId !== "";
  return action.connectorInstanceId !== "" && action.actionId !== "";
}

/** State selection and transitions, intentionally excluding display labels. */
export function isPlacementActionStateComplete(action: PlacementAction | null): boolean {
  if (action?.type !== "connectorAction") return true;
  if (action.stateDataPointId == null) return true;
  if (
    action.stateDataPointId === "" ||
    action.toTrue == null ||
    action.toFalse == null ||
    action.toTrue.actionId === "" ||
    action.toFalse.actionId === ""
  ) {
    return false;
  }
  return action.renderStyle === "switch" || action.renderStyle === "stateButton";
}

/** Whether the short third step has anything to configure. */
export function placementActionNeedsDisplayStep(action: PlacementAction | null): boolean {
  return (
    action?.type === "connectorAction" &&
    action.stateDataPointId != null &&
    action.renderStyle === "stateButton"
  );
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
