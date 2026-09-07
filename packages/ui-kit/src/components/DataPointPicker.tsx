import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
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
import type {
  DataPointDescriptor,
  ScreensaverDataPoint,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { matchesTarget } from "@loom/ui-kit/lib/connector-details";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

const HOST_TARGET = "__loom_host__";

type FixedDataPointContext = {
  connectorInstanceId: string;
  targetId: string | null;
  dataPoints: DataPointDescriptor[];
};

/**
 * Selects one connector instance, one of its addressable views, and one data
 * point from that view. PlacementBindingEditor supplies a fixed context and
 * reuses the final data-point control; kiosk configuration uses the complete
 * cascade. Keeping both paths here gives saved dashboard and screensaver
 * references the same target filtering and empty-selection behavior.
 */
export function DataPointPicker({
  value,
  onChange,
  fixedContext,
  disabled,
  idPrefix = "data-point",
  className,
}: {
  value: ScreensaverDataPoint | null;
  onChange: (value: ScreensaverDataPoint | null) => void;
  fixedContext?: FixedDataPointContext;
  disabled?: boolean;
  idPrefix?: string;
  className?: string;
}) {
  const api = useApiClient();
  const [draftInstanceId, setDraftInstanceId] = React.useState(
    fixedContext?.connectorInstanceId ?? value?.connectorInstanceId ?? "",
  );
  const [draftTargetId, setDraftTargetId] = React.useState<string | null>(
    fixedContext?.targetId ?? value?.targetId ?? null,
  );

  React.useEffect(() => {
    if (fixedContext !== undefined) {
      setDraftInstanceId(fixedContext.connectorInstanceId);
      setDraftTargetId(fixedContext.targetId);
      return;
    }
    if (value !== null) {
      setDraftInstanceId(value.connectorInstanceId);
      setDraftTargetId(value.targetId);
    }
  }, [fixedContext, value]);

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: fixedContext === undefined,
  });
  const detail = useQuery({
    queryKey: ["connector-instance", draftInstanceId],
    queryFn: ({ signal }) => api.getConnectorInstance(draftInstanceId, signal),
    enabled: fixedContext === undefined && draftInstanceId !== "",
  });
  const subTargets = useQuery({
    queryKey: ["connector-instance-sub-targets", draftInstanceId],
    queryFn: ({ signal }) => api.getSubTargets(draftInstanceId, signal),
    enabled:
      fixedContext === undefined &&
      draftInstanceId !== "" &&
      detail.data?.supportsSubTargets === true,
  });

  const dataPoints = fixedContext?.dataPoints ?? detail.data?.dataPoints ?? [];
  const availableDataPoints = dataPoints.filter((point) => matchesTarget(point, draftTargetId));
  const selectedDataPointId =
    value !== null &&
    value.connectorInstanceId === draftInstanceId &&
    value.targetId === draftTargetId
      ? value.dataPointId
      : "";

  function selectFirstPoint(instanceId: string, targetId: string | null, points: DataPointDescriptor[]) {
    const first = points.find((point) => matchesTarget(point, targetId));
    onChange(
      first === undefined
        ? null
        : { connectorInstanceId: instanceId, targetId, dataPointId: first.id },
    );
  }

  if (fixedContext !== undefined) {
    return (
      <DataPointSelect
        id={`${idPrefix}-point`}
        value={selectedDataPointId}
        dataPoints={availableDataPoints}
        disabled={disabled}
        onChange={(dataPointId) =>
          onChange({
            connectorInstanceId: fixedContext.connectorInstanceId,
            targetId: fixedContext.targetId,
            dataPointId,
          })
        }
      />
    );
  }

  return (
    <div className={cn("grid gap-3 sm:grid-cols-3", className)}>
      <div className="flex flex-col gap-2">
        <Label htmlFor={`${idPrefix}-instance`}>Connector</Label>
        <Select
          value={draftInstanceId}
          disabled={disabled || instances.isPending || instances.isError}
          onValueChange={(instanceId) => {
            if (instanceId === "") return;
            setDraftInstanceId(instanceId);
            setDraftTargetId(null);
            onChange(null);
          }}
        >
          <SelectTrigger id={`${idPrefix}-instance`}>
            <SelectValue placeholder="Choose a connector" />
          </SelectTrigger>
          <SelectContent>
            {(instances.data ?? []).map((instance) => (
              <SelectItem key={instance.id} value={instance.id}>
                {instance.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor={`${idPrefix}-target`}>View</Label>
        <Select
          value={draftTargetId ?? HOST_TARGET}
          disabled={disabled || draftInstanceId === "" || detail.isPending || detail.isError}
          onValueChange={(target) => {
            if (target === "") return;
            const targetId = target === HOST_TARGET ? null : target;
            setDraftTargetId(targetId);
            selectFirstPoint(draftInstanceId, targetId, dataPoints);
          }}
        >
          <SelectTrigger id={`${idPrefix}-target`}>
            <SelectValue placeholder="Choose a view" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={HOST_TARGET}>Server info</SelectItem>
            {(subTargets.data ?? []).map((target) => (
              <SelectItem key={target.id} value={target.id}>
                {target.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <DataPointSelect
        id={`${idPrefix}-point`}
        value={selectedDataPointId}
        dataPoints={availableDataPoints}
        disabled={disabled || detail.isPending || detail.isError}
        onChange={(dataPointId) =>
          onChange({ connectorInstanceId: draftInstanceId, targetId: draftTargetId, dataPointId })
        }
      />

      {instances.isPending || detail.isPending ? (
        <Skeleton className="h-9 w-full sm:col-span-3" />
      ) : null}
      {instances.isError || detail.isError || subTargets.isError ? (
        <Alert variant="destructive" className="sm:col-span-3">
          <AlertDescription>
            {describeConnectorError(instances.error ?? detail.error ?? subTargets.error)}
          </AlertDescription>
        </Alert>
      ) : null}
    </div>
  );
}

function DataPointSelect({
  id,
  value,
  dataPoints,
  disabled,
  onChange,
}: {
  id: string;
  value: string;
  dataPoints: DataPointDescriptor[];
  disabled?: boolean;
  onChange: (dataPointId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={id}>Data point</Label>
      <Select
        value={value}
        disabled={disabled || dataPoints.length === 0}
        onValueChange={(next) => next !== "" && onChange(next)}
      >
        <SelectTrigger id={id}>
          <SelectValue placeholder="Choose a data point" />
        </SelectTrigger>
        <SelectContent>
          {dataPoints.map((point) => (
            <SelectItem key={point.id} value={point.id}>
              {point.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

/** Ordered editor shared by kiosk creation and later user administration. */
export function ScreensaverConfigEditor({
  value,
  onChange,
  disabled,
}: {
  value: Array<ScreensaverDataPoint | null>;
  onChange: (value: Array<ScreensaverDataPoint | null>) => void;
  disabled?: boolean;
}) {
  const rowKeys = React.useRef<number[]>([]);
  const nextKey = React.useRef(0);
  while (rowKeys.current.length < value.length) rowKeys.current.push(nextKey.current++);
  rowKeys.current.length = value.length;

  function move(index: number, offset: -1 | 1) {
    const destination = index + offset;
    if (destination < 0 || destination >= value.length) return;
    const next = [...value];
    [next[index], next[destination]] = [next[destination], next[index]];
    [rowKeys.current[index], rowKeys.current[destination]] = [
      rowKeys.current[destination],
      rowKeys.current[index],
    ];
    onChange(next);
  }

  return (
    <div className="flex flex-col gap-3">
      {value.length === 0 ? (
        <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
          No rotating stats selected. The screensaver will show only the clock and date.
        </p>
      ) : (
        <ol className="flex flex-col gap-3">
          {value.map((selection, index) => (
            <li key={rowKeys.current[index]} className="surface-panel rounded-lg border p-3">
              <div className="mb-3 flex items-center justify-between gap-2">
                <span className="text-sm font-medium">Stat {index + 1}</span>
                <div className="flex gap-1">
                  <Button type="button" variant="ghost" size="icon" disabled={disabled || index === 0} onClick={() => move(index, -1)} aria-label={`Move stat ${index + 1} up`}>
                    <ArrowUp aria-hidden="true" />
                  </Button>
                  <Button type="button" variant="ghost" size="icon" disabled={disabled || index === value.length - 1} onClick={() => move(index, 1)} aria-label={`Move stat ${index + 1} down`}>
                    <ArrowDown aria-hidden="true" />
                  </Button>
                  <Button type="button" variant="ghost" size="icon" disabled={disabled} onClick={() => { rowKeys.current.splice(index, 1); onChange(value.filter((_, position) => position !== index)); }} aria-label={`Remove stat ${index + 1}`}>
                    <Trash2 aria-hidden="true" />
                  </Button>
                </div>
              </div>
              <DataPointPicker
                value={selection}
                onChange={(next) => onChange(value.map((entry, position) => position === index ? next : entry))}
                disabled={disabled}
                idPrefix={`screensaver-${rowKeys.current[index]}`}
              />
            </li>
          ))}
        </ol>
      )}
      <div>
        <Button type="button" variant="outline" size="sm" disabled={disabled} onClick={() => onChange([...value, null])}>
          <Plus aria-hidden="true" />
          Add stat
        </Button>
      </div>
    </div>
  );
}
