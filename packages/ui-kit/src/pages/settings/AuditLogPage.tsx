import * as React from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { AlertCircle, CalendarDays, Check, ChevronsUpDown, Loader2, X } from "lucide-react";

import { AuditLogTable } from "@loom/ui-kit/components/AuditLogTable";
import { ConnectorDetailModal } from "@loom/ui-kit/components/ConnectorDetailModal";
import { AUDIT_LOG_PAGE_SIZE } from "@loom/ui-kit/components/InstanceActionHistory";
import {
  SearchablePickerList,
  type SearchablePickerOption,
} from "@loom/ui-kit/components/SearchablePickerList";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@loom/ui-kit/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import type {
  AuditLogFilters,
  ConnectorInstanceSummary,
  DashboardPlacement,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/** Cross-instance action history for connector administrators. */
export function AuditLogPage() {
  const api = useApiClient();
  const [instanceId, setInstanceId] = React.useState<string | null>(null);
  const [userId, setUserId] = React.useState<string | null>(null);
  const [actionId, setActionId] = React.useState("");
  const [outcome, setOutcome] = React.useState("all");
  const [afterDate, setAfterDate] = React.useState("");
  const [beforeDate, setBeforeDate] = React.useState("");
  const [openedInstanceId, setOpenedInstanceId] = React.useState<string | null>(null);
  const deferredActionId = React.useDeferredValue(actionId.trim());

  // Independent reference lists start together. Either may be forbidden for a
  // narrowly granted connector manager; the audit table itself remains usable
  // and only that searchable filter is disabled.
  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    retry: false,
  });
  const users = useQuery({
    queryKey: ["users"],
    queryFn: ({ signal }) => api.getUsers(signal),
    retry: false,
  });

  const filters = React.useMemo<AuditLogFilters>(
    () => ({
      instanceId: instanceId ?? undefined,
      userId: userId ?? undefined,
      actionId: deferredActionId || undefined,
      success: outcome === "all" ? undefined : outcome === "success",
      after: startOfDay(afterDate),
      before: dayAfter(beforeDate),
      limit: AUDIT_LOG_PAGE_SIZE,
    }),
    [afterDate, beforeDate, deferredActionId, instanceId, outcome, userId],
  );

  const auditLog = useInfiniteQuery({
    queryKey: ["global-audit-log", filters],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      api.getGlobalAuditLog({ ...filters, before: pageParam ?? filters.before }, signal),
    getNextPageParam: (lastPage) =>
      lastPage.length < AUDIT_LOG_PAGE_SIZE
        ? undefined
        : lastPage[lastPage.length - 1]?.invokedAt,
  });

  const entries = React.useMemo(
    () => auditLog.data?.pages.flat() ?? [],
    [auditLog.data?.pages],
  );
  const instanceOptions = React.useMemo<SearchablePickerOption[]>(
    () =>
      (instances.data ?? []).map((instance) => ({
        id: instance.id,
        label: instance.name,
        badge: instance.connectorType,
      })),
    [instances.data],
  );
  const userOptions = React.useMemo<SearchablePickerOption[]>(
    () =>
      (users.data ?? []).map((user) => ({
        id: user.id,
        label: user.username,
        badge: user.isActive ? undefined : "inactive",
      })),
    [users.data],
  );
  const openedInstance = (instances.data ?? []).find(
    (instance) => instance.id === openedInstanceId,
  );
  const hasFilters =
    instanceId !== null ||
    userId !== null ||
    actionId !== "" ||
    outcome !== "all" ||
    afterDate !== "" ||
    beforeDate !== "";

  const clearFilters = () => {
    setInstanceId(null);
    setUserId(null);
    setActionId("");
    setOutcome("all");
    setAfterDate("");
    setBeforeDate("");
  };

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Audit Log</CardTitle>
          <CardDescription>
            Connector actions across every instance, including failures and system-run updates.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="grid gap-3 lg:grid-cols-3">
            <SearchableFilter
              label="Instance"
              placeholder={instances.isError ? "Instance list unavailable" : "All instances"}
              options={instanceOptions}
              selectedId={instanceId}
              disabled={instances.isError}
              onSelect={setInstanceId}
            />
            <SearchableFilter
              label="User"
              placeholder={users.isError ? "User list unavailable" : "All users"}
              options={userOptions}
              selectedId={userId}
              disabled={users.isError}
              onSelect={setUserId}
            />
            <div className="flex flex-col gap-2">
              <Label htmlFor="audit-action-filter">Action</Label>
              <Input
                id="audit-action-filter"
                value={actionId}
                placeholder="Action id"
                onChange={(event) => setActionId(event.target.value)}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label>Outcome</Label>
              <Select value={outcome} onValueChange={setOutcome}>
                <SelectTrigger aria-label="Filter audit log by outcome">
                  <SelectValue placeholder="All outcomes" />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectItem value="all">All outcomes</SelectItem>
                    <SelectItem value="success">Successful</SelectItem>
                    <SelectItem value="failure">Failed</SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
            </div>
            <DateRangeFilter
              afterDate={afterDate}
              beforeDate={beforeDate}
              onAfterDateChange={setAfterDate}
              onBeforeDateChange={setBeforeDate}
            />
            <div className="flex items-end">
              {hasFilters ? (
                <Button type="button" variant="ghost" onClick={clearFilters}>
                  <X data-icon="inline-start" aria-hidden="true" />
                  Clear filters
                </Button>
              ) : null}
            </div>
          </div>

          {auditLog.isPending ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-40 w-full" />
            </div>
          ) : auditLog.isError ? (
            <Alert variant="destructive">
              <AlertCircle aria-hidden="true" />
              <AlertTitle>Could not load the audit log</AlertTitle>
              <AlertDescription>{describeConnectorError(auditLog.error)}</AlertDescription>
            </Alert>
          ) : entries.length === 0 ? (
            <p className="py-10 text-center text-sm text-muted-foreground">
              No connector actions match these filters.
            </p>
          ) : (
            <AuditLogTable
              entries={entries}
              showInstance
              onOpenInstance={instances.data ? setOpenedInstanceId : undefined}
            />
          )}

          {auditLog.hasNextPage ? (
            <Button
              type="button"
              variant="outline"
              className="self-center"
              disabled={auditLog.isFetchingNextPage}
              onClick={() => void auditLog.fetchNextPage()}
            >
              {auditLog.isFetchingNextPage ? (
                <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
              ) : null}
              Load more
            </Button>
          ) : null}
        </CardContent>
      </Card>

      {openedInstance ? (
        <ConnectorDetailModal
          placement={auditPlacement(openedInstance)}
          open
          initialTab="history"
          onOpenChange={(open) => {
            if (!open) setOpenedInstanceId(null);
          }}
        />
      ) : null}
    </>
  );
}

function SearchableFilter({
  label,
  placeholder,
  options,
  selectedId,
  disabled,
  onSelect,
}: {
  label: string;
  placeholder: string;
  options: SearchablePickerOption[];
  selectedId: string | null;
  disabled: boolean;
  onSelect: (id: string | null) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const selected = options.find((option) => option.id === selectedId);
  return (
    <div className="flex flex-col gap-2">
      <Label>{label}</Label>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            className="justify-between font-normal"
            disabled={disabled}
          >
            <span className="truncate">{selected?.label ?? placeholder}</span>
            <ChevronsUpDown aria-hidden="true" />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="flex max-h-80 w-80 flex-col gap-2 p-3">
          {selectedId ? (
            <Button
              type="button"
              variant="ghost"
              className="justify-start"
              onClick={() => {
                onSelect(null);
                setOpen(false);
              }}
            >
              <Check data-icon="inline-start" aria-hidden="true" />
              Show all
            </Button>
          ) : null}
          <SearchablePickerList
            options={options}
            searchLabel={`Search ${label.toLocaleLowerCase()}`}
            selectedId={selectedId}
            onSelect={(id) => {
              onSelect(id);
              setOpen(false);
            }}
          />
        </PopoverContent>
      </Popover>
    </div>
  );
}

function DateRangeFilter({
  afterDate,
  beforeDate,
  onAfterDateChange,
  onBeforeDateChange,
}: {
  afterDate: string;
  beforeDate: string;
  onAfterDateChange: (value: string) => void;
  onBeforeDateChange: (value: string) => void;
}) {
  const active = afterDate !== "" || beforeDate !== "";
  return (
    <div className="flex flex-col gap-2">
      <Label>Date range</Label>
      <Popover>
        <PopoverTrigger asChild>
          <Button type="button" variant="outline" className="justify-start font-normal">
            <CalendarDays data-icon="inline-start" aria-hidden="true" />
            {active ? `${afterDate || "…"} – ${beforeDate || "…"}` : "Any date"}
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="flex w-72 flex-col gap-3">
          <p className="text-sm font-medium">Date range</p>
          <div className="flex flex-col gap-2">
            <Label htmlFor="audit-after-date">From</Label>
            <Input
              id="audit-after-date"
              type="text"
              inputMode="numeric"
              placeholder="YYYY-MM-DD"
              value={afterDate}
              onChange={(event) => onAfterDateChange(validDateText(event.target.value))}
            />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="audit-before-date">Through</Label>
            <Input
              id="audit-before-date"
              type="text"
              inputMode="numeric"
              placeholder="YYYY-MM-DD"
              value={beforeDate}
              onChange={(event) => onBeforeDateChange(validDateText(event.target.value))}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            Dates use YYYY-MM-DD and include the entire final day.
          </p>
        </PopoverContent>
      </Popover>
    </div>
  );
}

function validDateText(value: string): string {
  return value.replace(/[^0-9-]/g, "").slice(0, 10);
}

function startOfDay(value: string): string | undefined {
  return parseExactDate(value) === null ? undefined : `${value}T00:00:00+00:00`;
}

function dayAfter(value: string): string | undefined {
  const date = parseExactDate(value);
  if (date === null) return undefined;
  date.setUTCDate(date.getUTCDate() + 1);
  return date.toISOString().replace("Z", "+00:00");
}

function parseExactDate(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(Date.UTC(year, month - 1, day));
  return date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
    ? date
    : null;
}

function auditPlacement(
  instance: ConnectorInstanceSummary,
): DashboardPlacement & { connector: ConnectorInstanceSummary } {
  return {
    id: `audit:${instance.id}`,
    connector: instance,
    targetId: null,
    positionX: 0,
    positionY: 0,
    width: instance.metadata.minSize[0],
    height: instance.metadata.minSize[1],
    widgetBindings: [],
    // A synthetic placement, so it has none of the things a real one carries:
    // nothing clicks it, and its name comes from the connector.
    placementAction: null,
    label: null,
    icon: null,
    createdAt: instance.createdAt,
    groupId: null,
  };
}
