import * as React from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
} from "@tanstack/react-query";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  Loader2,
  Pencil,
  Plug,
  Plus,
  ScanSearch,
  Search,
  Trash2,
  XCircle,
} from "lucide-react";
import { toast } from "sonner";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@loom/ui-kit/components/ui/alert-dialog";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { Card, CardContent } from "@loom/ui-kit/components/ui/card";
import { Checkbox } from "@loom/ui-kit/components/ui/checkbox";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { ConnectorInstanceDialog } from "@loom/ui-kit/components/ConnectorInstanceDialog";
import { DiscoverResourcesDialog } from "@loom/ui-kit/components/DiscoverResourcesDialog";
import { Input } from "@loom/ui-kit/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@loom/ui-kit/components/ui/select";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@loom/ui-kit/components/ui/table";
import {
  ApiError,
  SessionExpiredError,
  type ConnectorInstanceDetail,
  type ConnectorInstanceSummary,
  type ConnectorTypeSummary,
} from "@loom/ui-kit/lib/api";
import { useApiClient, useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { connectorAvailability } from "@loom/ui-kit/lib/connector-availability";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";
import { cn } from "@loom/ui-kit/lib/utils";

type ConnectorSort = "name-asc" | "name-desc" | "status";

type ConnectorGroup = {
  typeId: string;
  displayName: string;
  icon: string | null;
  instances: ConnectorInstanceSummary[];
};

type DeleteResult =
  | { state: "pending" }
  | { state: "success" }
  | { state: "failure"; message: string };

type DeleteSummary = { successes: number; attempted: number };

const STATUS_ORDER = {
  down: 0,
  degraded: 1,
  unknown: 2,
  pending: 3,
  healthy: 4,
} as const;

/** Connector administration table shared by Web, Desktop, and Mobile. */
export function ConnectorsView({
  renderShell,
}: {
  renderShell: (content: React.ReactNode) => React.ReactNode;
}) {
  const api = useApiClient();
  const connectorSocket = useConnectorStatusSocket();
  const queryClient = useQueryClient();
  const { isAuthenticated, signOut, user } = useAuth();
  const canManage = hasPermission(user?.permissions ?? [], PERMISSION_KEYS.connectorsManage);

  const instances = useQuery({
    queryKey: ["connector-instances"],
    queryFn: ({ signal }) => api.getConnectorInstances(signal),
    enabled: isAuthenticated,
    retry: (failureCount, error) =>
      !(error instanceof ApiError && error.isUnauthorized) &&
      !(error instanceof SessionExpiredError) &&
      failureCount < 2,
  });

  const tags = useQuery({
    queryKey: ["connector-tags"],
    queryFn: ({ signal }) => api.getConnectorTags(signal),
    enabled: isAuthenticated,
    retry: false,
  });

  const connectorTypes = useQuery({
    queryKey: ["connector-types"],
    queryFn: ({ signal }) => api.getConnectorTypes(signal),
    enabled: isAuthenticated && canManage,
    staleTime: Infinity,
    retry: false,
  });

  const instanceIds = React.useMemo(
    () => instances.data?.map((instance) => instance.id) ?? [],
    [instances.data],
  );

  React.useEffect(() => {
    if (!isAuthenticated || instanceIds.length === 0) return;

    return connectorSocket.subscribe(instanceIds, (update) => {
      queryClient.setQueryData<ConnectorInstanceSummary[]>(
        ["connector-instances"],
        (current) =>
          current?.map((instance) =>
            instance.id === update.instanceId
              ? {
                  ...instance,
                  status: update.status,
                  statusError: update.statusError,
                  pendingOperation: update.pendingOperation,
                  diagnosis: update.diagnosis,
                }
              : instance,
          ),
      );
      queryClient.setQueryData<ConnectorInstanceDetail>(
        ["connector-instance", update.instanceId],
        (current) =>
          current === undefined
            ? undefined
            : {
                ...current,
                status: update.status,
                statusError: update.statusError,
                pendingOperation: update.pendingOperation,
                diagnosis: update.diagnosis,
              },
      );
    });
  }, [connectorSocket, instanceIds, isAuthenticated, queryClient]);

  const [createOpen, setCreateOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<ConnectorInstanceSummary | null>(null);
  const [deleting, setDeleting] = React.useState<ConnectorInstanceSummary | null>(null);
  const [discovering, setDiscovering] = React.useState<ConnectorInstanceSummary | null>(null);
  const [searchText, setSearchText] = React.useState("");
  const deferredSearchText = React.useDeferredValue(searchText);
  const [activeTags, setActiveTags] = React.useState<Set<string>>(() => new Set());
  const [sort, setSort] = React.useState<ConnectorSort>("name-asc");
  const [selectMode, setSelectMode] = React.useState(false);
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(() => new Set());
  const [collapsedTypes, setCollapsedTypes] = React.useState<Set<string>>(() => new Set());
  const [bulkTargets, setBulkTargets] = React.useState<ConnectorInstanceSummary[]>([]);

  const invalidate = React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["connector-instances"] }),
      queryClient.invalidateQueries({ queryKey: ["connector-tags"] }),
    ]);
  }, [queryClient]);

  const removeInstance = useMutation({
    mutationFn: (target: ConnectorInstanceSummary) => api.deleteConnectorInstance(target.id),
    onSuccess: async (_result, target) => {
      setDeleting(null);
      toast.success(`Deleted ${target.name}.`);
      await invalidate();
      queryClient.removeQueries({ queryKey: ["connector-instance", target.id] });
    },
    onError: (error: unknown) => {
      const failure = describeAdminFailure(error);
      toast.error(
        failure.kind === "refused" ? "That deletion was refused" : "Could not delete connector",
        { description: failure.message, duration: 10_000 },
      );
    },
  });

  const isUnauthorized =
    instances.error instanceof SessionExpiredError ||
    (instances.error instanceof ApiError && instances.error.isUnauthorized);

  React.useEffect(() => {
    if (isUnauthorized) void signOut();
  }, [isUnauthorized, signOut]);

  const typesById = React.useMemo(
    () => new Map((connectorTypes.data ?? []).map((type) => [type.typeId, type])),
    [connectorTypes.data],
  );

  const groups = React.useMemo(
    () =>
      groupConnectors(
        instances.data ?? [],
        typesById,
        deferredSearchText,
        activeTags,
        sort,
      ),
    [activeTags, deferredSearchText, instances.data, sort, typesById],
  );
  const visibleInstances = React.useMemo(
    () => groups.flatMap((group) => group.instances),
    [groups],
  );
  const visibleIds = React.useMemo(
    () => visibleInstances.map((instance) => instance.id),
    [visibleInstances],
  );
  const visibleIdKey = visibleIds.join("|");

  React.useEffect(() => {
    const visible = new Set(visibleIds);
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => visible.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [visibleIdKey]);

  const allVisibleSelected =
    visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  const someVisibleSelected = visibleIds.some((id) => selectedIds.has(id));
  const hasFilters = searchText.trim() !== "" || activeTags.size > 0;
  const isEmpty = instances.isSuccess && instances.data.length === 0;

  function setTagFilter(tag: string, selected: boolean) {
    setActiveTags((current) => {
      const next = new Set(current);
      if (selected) next.add(tag);
      else next.delete(tag);
      return next;
    });
  }

  function setRowSelected(id: string, selected: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  function setVisibleSelected(selected: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const id of visibleIds) {
        if (selected) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }

  async function instanceDeleted(target: ConnectorInstanceSummary) {
    setSelectedIds((current) => {
      const next = new Set(current);
      next.delete(target.id);
      return next;
    });
    await invalidate();
    queryClient.removeQueries({ queryKey: ["connector-instance", target.id] });
  }

  return renderShell(
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Connectors</h1>
          <p className="text-sm text-muted-foreground">
            Services Loom is managing, grouped by connector type.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant={selectMode ? "secondary" : "outline"}
            size="sm"
            aria-pressed={selectMode}
            onClick={() => {
              setSelectMode((current) => !current);
              setSelectedIds(new Set());
            }}
          >
            Select
          </Button>
          {canManage ? (
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              <Plus aria-hidden="true" />
              Add connector
            </Button>
          ) : null}
        </div>
      </div>

      {instances.isPending ? <ConnectorTableSkeleton selectMode={selectMode} /> : null}

      {instances.isError ? (
        <Alert variant="destructive">
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Could not load connectors</AlertTitle>
          <AlertDescription>{describeConnectorError(instances.error)}</AlertDescription>
        </Alert>
      ) : null}

      {tags.isError ? (
        <Alert>
          <AlertCircle aria-hidden="true" />
          <AlertTitle>Tag filters unavailable</AlertTitle>
          <AlertDescription>
            Connector rows still show their assigned tags, but the shared filter list could not be loaded.
          </AlertDescription>
        </Alert>
      ) : null}

      {isEmpty ? <EmptyState canManage={canManage} onAdd={() => setCreateOpen(true)} /> : null}

      {instances.isSuccess && instances.data.length > 0 ? (
        <>
          <ConnectorFilters
            searchText={searchText}
            onSearchTextChange={setSearchText}
            tags={tags.data ?? []}
            activeTags={activeTags}
            onTagFilterChange={setTagFilter}
            sort={sort}
            onSortChange={setSort}
            hasFilters={hasFilters}
            onClear={() => {
              setSearchText("");
              setActiveTags(new Set());
            }}
          />

          {selectMode && selectedIds.size > 0 ? (
            <div className="surface-elevated sticky top-2 flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3 shadow-md">
              <p className="text-sm font-medium">{selectedIds.size} selected</p>
              <div className="flex flex-wrap gap-2">
                <Button type="button" variant="ghost" size="sm" onClick={() => setSelectedIds(new Set())}>
                  Clear selection
                </Button>
                {canManage ? (
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={() => {
                      setBulkTargets(
                        visibleInstances.filter((instance) => selectedIds.has(instance.id)),
                      );
                    }}
                  >
                    <Trash2 aria-hidden="true" />
                    Delete selected
                  </Button>
                ) : null}
              </div>
            </div>
          ) : null}

          <Card>
            <CardContent className="p-2">
              <Table>
                <TableHeader>
                  <TableRow>
                    {selectMode ? (
                      <TableHead className="w-10 pl-3">
                        <Checkbox
                          checked={
                            allVisibleSelected
                              ? true
                              : someVisibleSelected
                                ? "indeterminate"
                                : false
                          }
                          disabled={visibleIds.length === 0}
                          aria-label="Select all visible connectors"
                          onCheckedChange={(checked) => setVisibleSelected(checked === true)}
                        />
                      </TableHead>
                    ) : null}
                    <TableHead className={cn(!selectMode && "pl-3")}>Name</TableHead>
                    <TableHead>Tags</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Last checked</TableHead>
                    <TableHead className="w-32 pr-3 text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {groups.length === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={selectMode ? 6 : 5}
                        className="py-8 text-center text-muted-foreground"
                      >
                        No connectors match the current filters.
                      </TableCell>
                    </TableRow>
                  ) : (
                    groups.map((group) => {
                      const collapsed = collapsedTypes.has(group.typeId);
                      return (
                        <React.Fragment key={group.typeId}>
                          <TableRow className="bg-muted/40 hover:bg-muted/40">
                            <TableCell colSpan={selectMode ? 6 : 5} className="p-1">
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                className="w-full justify-start gap-2"
                                aria-expanded={!collapsed}
                                onClick={() => {
                                  setCollapsedTypes((current) => {
                                    const next = new Set(current);
                                    if (collapsed) next.delete(group.typeId);
                                    else next.add(group.typeId);
                                    return next;
                                  });
                                }}
                              >
                                <ChevronDown
                                  aria-hidden="true"
                                  className={cn("transition-transform", collapsed && "-rotate-90")}
                                />
                                <ConnectorIcon typeIcon={group.icon} iconOverride={null} size={18} />
                                <span className="font-semibold">{group.displayName}</span>
                                <Badge variant="secondary" className="ml-auto">
                                  {group.instances.length}
                                </Badge>
                              </Button>
                            </TableCell>
                          </TableRow>
                          {collapsed
                            ? null
                            : group.instances.map((instance) => (
                                <ConnectorTableRow
                                  key={instance.id}
                                  instance={instance}
                                  selectMode={selectMode}
                                  selected={selectedIds.has(instance.id)}
                                  canManage={canManage}
                                  type={typesById.get(instance.connectorType)}
                                  onSelectedChange={(selected) =>
                                    setRowSelected(instance.id, selected)
                                  }
                                  onDiscover={setDiscovering}
                                  onEdit={setEditing}
                                  onDelete={setDeleting}
                                />
                              ))}
                        </React.Fragment>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </>
      ) : null}

      <ConnectorInstanceDialog
        key={editing?.id ?? "create"}
        open={createOpen || editing !== null}
        instance={editing}
        onOpenChange={(open) => {
          if (!open) {
            setCreateOpen(false);
            setEditing(null);
          }
        }}
        onSaved={invalidate}
      />

      {discovering !== null ? (
        <DiscoverResourcesDialog
          open
          instanceId={discovering.id}
          instanceName={discovering.name}
          onOpenChange={(open) => {
            if (!open) setDiscovering(null);
          }}
          onCreated={invalidate}
        />
      ) : null}

      <SingleDeleteDialog
        deleting={deleting}
        mutation={removeInstance}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      />

      <BulkDeleteDialog
        open={bulkTargets.length > 0}
        targets={bulkTargets}
        onOpenChange={(open) => {
          if (!open) setBulkTargets([]);
        }}
        onDeleted={instanceDeleted}
      />
    </div>,
  );
}

function ConnectorFilters({
  searchText,
  onSearchTextChange,
  tags,
  activeTags,
  onTagFilterChange,
  sort,
  onSortChange,
  hasFilters,
  onClear,
}: {
  searchText: string;
  onSearchTextChange: (value: string) => void;
  tags: string[];
  activeTags: Set<string>;
  onTagFilterChange: (tag: string, selected: boolean) => void;
  sort: ConnectorSort;
  onSortChange: (sort: ConnectorSort) => void;
  hasFilters: boolean;
  onClear: () => void;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-2 sm:flex-row">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            value={searchText}
            className="pl-9"
            placeholder="Search names and tags"
            aria-label="Search connectors"
            onChange={(event) => onSearchTextChange(event.target.value)}
          />
        </div>
        <Select value={sort} onValueChange={(value) => onSortChange(value as ConnectorSort)}>
          <SelectTrigger className="w-full sm:w-44" aria-label="Sort connectors">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="name-asc">Name A→Z</SelectItem>
              <SelectItem value="name-desc">Name Z→A</SelectItem>
              <SelectItem value="status">Status</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      </div>

      {tags.length > 0 || hasFilters ? (
        <div className="flex flex-wrap items-center gap-2">
          {tags.map((tag) => {
            const selected = activeTags.has(tag);
            return (
              <Button
                key={tag}
                type="button"
                variant={selected ? "secondary" : "outline"}
                size="sm"
                className="rounded-full"
                aria-pressed={selected}
                onClick={() => onTagFilterChange(tag, !selected)}
              >
                {tag}
              </Button>
            );
          })}
          {hasFilters ? (
            <Button type="button" variant="ghost" size="sm" onClick={onClear}>
              Clear filters
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function ConnectorTableRow({
  instance,
  selectMode,
  selected,
  canManage,
  type,
  onSelectedChange,
  onDiscover,
  onEdit,
  onDelete,
}: {
  instance: ConnectorInstanceSummary;
  selectMode: boolean;
  selected: boolean;
  canManage: boolean;
  type: ConnectorTypeSummary | undefined;
  onSelectedChange: (selected: boolean) => void;
  onDiscover: (instance: ConnectorInstanceSummary) => void;
  onEdit: (instance: ConnectorInstanceSummary) => void;
  onDelete: (instance: ConnectorInstanceSummary) => void;
}) {
  const availability = connectorAvailability(instance);
  const canDiscover = canManage && type?.discoverableType != null;

  return (
    <TableRow data-state={selected ? "selected" : undefined}>
      {selectMode ? (
        <TableCell className="w-10 pl-3">
          <Checkbox
            checked={selected}
            aria-label={`${selected ? "Deselect" : "Select"} ${instance.name}`}
            onCheckedChange={(checked) => onSelectedChange(checked === true)}
          />
        </TableCell>
      ) : null}
      <TableCell className={cn("font-medium", !selectMode && "pl-3")}>
        <div className="flex min-w-40 items-center gap-2">
          <ConnectorIcon
            typeIcon={instance.metadata.icon}
            iconOverride={instance.iconOverride}
            size={24}
          />
          <span className="truncate">{instance.name}</span>
        </div>
      </TableCell>
      <TableCell>
        <TagBadges tags={instance.tags} />
      </TableCell>
      <TableCell>
        <Badge variant={availability.tone}>{availability.label}</Badge>
      </TableCell>
      <TableCell className="whitespace-nowrap text-muted-foreground">
        {instance.status === null ? (
          "—"
        ) : (
          <time dateTime={instance.status.lastChecked}>{formatChecked(instance.status.lastChecked)}</time>
        )}
      </TableCell>
      <TableCell className="pr-3 text-right">
        {canManage ? (
          <div className="flex justify-end gap-1">
            {canDiscover ? (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Discover resources through ${instance.name}`}
                onClick={() => onDiscover(instance)}
              >
                <ScanSearch aria-hidden="true" />
              </Button>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={`Edit ${instance.name}`}
              onClick={() => onEdit(instance)}
            >
              <Pencil aria-hidden="true" />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={`Delete ${instance.name}`}
              onClick={() => onDelete(instance)}
            >
              <Trash2 aria-hidden="true" />
            </Button>
          </div>
        ) : null}
      </TableCell>
    </TableRow>
  );
}

function TagBadges({ tags }: { tags: string[] }) {
  if (tags.length === 0) return <span className="text-muted-foreground">—</span>;
  const visible = tags.slice(0, 2);
  return (
    <div className="flex min-w-36 flex-wrap gap-1">
      {visible.map((tag) => (
        <Badge key={tag} variant="outline">
          {tag}
        </Badge>
      ))}
      {tags.length > visible.length ? (
        <Badge variant="secondary" aria-label={`${tags.length - visible.length} more tags`}>
          +{tags.length - visible.length} more
        </Badge>
      ) : null}
    </div>
  );
}

function SingleDeleteDialog({
  deleting,
  mutation,
  onOpenChange,
}: {
  deleting: ConnectorInstanceSummary | null;
  mutation: UseMutationResult<void, unknown, ConnectorInstanceSummary>;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <AlertDialog open={deleting !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {deleting?.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            Loom stops managing this service and forgets its configuration. The service itself is
            not affected. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={mutation.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            disabled={mutation.isPending}
            onClick={(event) => {
              event.preventDefault();
              if (deleting !== null) mutation.mutate(deleting);
            }}
          >
            {mutation.isPending ? <Loader2 className="animate-spin" aria-hidden="true" /> : null}
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function BulkDeleteDialog({
  open,
  targets,
  onOpenChange,
  onDeleted,
}: {
  open: boolean;
  targets: ConnectorInstanceSummary[];
  onOpenChange: (open: boolean) => void;
  onDeleted: (target: ConnectorInstanceSummary) => Promise<void>;
}) {
  const api = useApiClient();
  const [results, setResults] = React.useState<Record<string, DeleteResult>>({});
  const [summary, setSummary] = React.useState<DeleteSummary | null>(null);
  const [isDeleting, setIsDeleting] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setResults({});
      setSummary(null);
    }
  }, [open, targets]);

  const failedTargets = targets.filter((target) => results[target.id]?.state === "failure");
  const remainingTargets = summary === null ? targets : failedTargets;

  async function deleteTargets() {
    if (remainingTargets.length === 0) return;
    setIsDeleting(true);
    setSummary(null);
    const successfulIds = new Set(
      targets
        .filter((target) => results[target.id]?.state === "success")
        .map((target) => target.id),
    );

    for (const target of remainingTargets) {
      setResults((current) => ({ ...current, [target.id]: { state: "pending" } }));
      try {
        await api.deleteConnectorInstance(target.id);
        successfulIds.add(target.id);
        setResults((current) => ({ ...current, [target.id]: { state: "success" } }));
        await onDeleted(target);
      } catch (error: unknown) {
        setResults((current) => ({
          ...current,
          [target.id]: { state: "failure", message: describeAdminFailure(error).message },
        }));
      }
    }

    setSummary({ successes: successfulIds.size, attempted: targets.length });
    setIsDeleting(false);
  }

  const allSucceeded =
    summary !== null && targets.every((target) => results[target.id]?.state === "success");

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!isDeleting) onOpenChange(next);
      }}
    >
      <AlertDialogContent className="max-h-[85vh] overflow-y-auto">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {targets.length} selected connectors?</AlertDialogTitle>
          <AlertDialogDescription>
            Loom stops managing these services and forgets their configurations. The services
            themselves are not affected. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="flex flex-col gap-2">
          {targets.map((target) => (
            <div key={target.id} className="surface-panel rounded-md border p-3">
              <p className="font-medium">{target.name}</p>
              <DeleteResultView result={results[target.id]} />
            </div>
          ))}
        </div>

        {summary !== null ? (
          <Alert variant={failedTargets.length > 0 ? "destructive" : undefined}>
            {failedTargets.length > 0 ? <XCircle aria-hidden="true" /> : <CheckCircle2 aria-hidden="true" />}
            <AlertTitle>Bulk deletion complete</AlertTitle>
            <AlertDescription>
              {summary.successes} of {summary.attempted} deleted successfully.
              {failedTargets.length > 0 ? " Failed rows remain available to retry." : ""}
            </AlertDescription>
          </Alert>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogCancel disabled={isDeleting}>
            {summary !== null ? "Close" : "Cancel"}
          </AlertDialogCancel>
          {!allSucceeded ? (
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={isDeleting || remainingTargets.length === 0}
              onClick={(event) => {
                event.preventDefault();
                void deleteTargets();
              }}
            >
              {isDeleting ? <Loader2 className="animate-spin" aria-hidden="true" /> : <Trash2 aria-hidden="true" />}
              {summary === null ? "Delete selected" : "Retry failed"}
            </AlertDialogAction>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function DeleteResultView({ result }: { result: DeleteResult | undefined }) {
  if (result === undefined) return null;
  if (result.state === "pending") {
    return (
      <p className="mt-1 flex items-center gap-1 text-xs text-muted-foreground" aria-live="polite">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
        Deleting…
      </p>
    );
  }
  if (result.state === "success") {
    return (
      <p className="mt-1 flex items-center gap-1 text-xs font-medium text-primary" aria-live="polite">
        <CheckCircle2 className="size-3.5" aria-hidden="true" />
        Deleted
      </p>
    );
  }
  return (
    <p className="mt-1 flex items-start gap-1 text-xs text-destructive" aria-live="polite">
      <XCircle className="size-3.5 shrink-0" aria-hidden="true" />
      {result.message}
    </p>
  );
}

function groupConnectors(
  instances: ConnectorInstanceSummary[],
  typesById: Map<string, ConnectorTypeSummary>,
  searchText: string,
  activeTags: Set<string>,
  sort: ConnectorSort,
): ConnectorGroup[] {
  const needle = searchText.trim().toLocaleLowerCase();
  const grouped = new Map<string, ConnectorGroup>();

  for (const instance of instances) {
    const matchesSearch =
      needle === "" ||
      instance.name.toLocaleLowerCase().includes(needle) ||
      instance.tags.some((tag) => tag.toLocaleLowerCase().includes(needle));
    const matchesTags =
      activeTags.size === 0 || instance.tags.some((tag) => activeTags.has(tag));
    if (!matchesSearch || !matchesTags) continue;

    const catalogType = typesById.get(instance.connectorType);
    const existing = grouped.get(instance.connectorType);
    if (existing === undefined) {
      grouped.set(instance.connectorType, {
        typeId: instance.connectorType,
        displayName: catalogType?.displayName ?? instance.metadata.name,
        icon: catalogType?.icon ?? instance.metadata.icon,
        instances: [instance],
      });
    } else {
      existing.instances.push(instance);
    }
  }

  const compareName = (left: ConnectorInstanceSummary, right: ConnectorInstanceSummary) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
  for (const group of grouped.values()) {
    group.instances.sort((left, right) => {
      if (sort === "name-asc") return compareName(left, right);
      if (sort === "name-desc") return compareName(right, left);
      const statusDifference =
        STATUS_ORDER[connectorAvailability(left).tone] -
        STATUS_ORDER[connectorAvailability(right).tone];
      return statusDifference === 0 ? compareName(left, right) : statusDifference;
    });
  }

  return [...grouped.values()].sort((left, right) =>
    left.displayName.localeCompare(right.displayName, undefined, { sensitivity: "base" }),
  );
}

function formatChecked(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function EmptyState({ canManage, onAdd }: { canManage: boolean; onAdd: () => void }) {
  return (
    <Card className="surface-elevated">
      <CardContent className="flex flex-col items-center gap-3 py-10 text-center">
        <Plug className="size-8 text-muted-foreground" aria-hidden="true" />
        <div className="flex flex-col gap-1">
          <p className="font-medium">No connectors yet</p>
          <p className="max-w-md text-sm text-muted-foreground">
            {canManage
              ? "A connector is Loom's link to one service it manages. Add one to see its status and act on it."
              : "Nothing has been added to this instance yet. Someone with the connectors.manage permission can add one."}
          </p>
        </div>
        {canManage ? (
          <Button onClick={onAdd}>
            <Plus aria-hidden="true" />
            Add connector
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ConnectorTableSkeleton({ selectMode }: { selectMode: boolean }) {
  return (
    <Card>
      <CardContent className="p-2">
        <Table>
          <TableBody>
            {[0, 1, 2].map((row) => (
              <TableRow key={row}>
                {selectMode ? (
                  <TableCell>
                    <Skeleton className="size-4" />
                  </TableCell>
                ) : null}
                {[0, 1, 2, 3, 4].map((column) => (
                  <TableCell key={column}>
                    <Skeleton className="h-4 w-24" />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}
