import * as React from "react";
import { DndContext, KeyboardSensor, PointerSensor, closestCenter, useDroppable, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, arrayMove, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Check, ChevronDown, FolderPlus, GripVertical, MoreHorizontal, Pencil, Plus } from "lucide-react";
import { toast } from "sonner";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@loom/ui-kit/components/ui/alert-dialog";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import { GenericIconPicker } from "@loom/ui-kit/components/GenericIconPicker";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@loom/ui-kit/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger } from "@loom/ui-kit/components/ui/dropdown-menu";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@loom/ui-kit/components/ui/tooltip";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import type { DashboardFolder, DashboardSummary } from "@loom/ui-kit/lib/api";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

export const dashboardsQueryKey = ["dashboards"] as const;
export const dashboardFoldersQueryKey = ["dashboard-folders"] as const;

type ContainerKey = `folder:${string}` | "ungrouped:owner" | "ungrouped:shared";
const folderDragId = (id: string) => `folder-sort:${id}`;
const dashboardDragId = (id: string) => `dashboard-sort:${id}`;
const folderId = (container: ContainerKey) => container.startsWith("folder:") ? container.slice(7) : null;
const dashboardContainer = (dashboard: DashboardSummary): ContainerKey => dashboard.sidebarFolderId ? `folder:${dashboard.sidebarFolderId}` : dashboard.role === "owner" ? "ungrouped:owner" : "ungrouped:shared";
const ordered = (dashboards: DashboardSummary[], container: ContainerKey) => dashboards.filter((dashboard) => dashboardContainer(dashboard) === container).sort((a, b) => a.sidebarSortOrder - b.sidebarSortOrder || a.name.localeCompare(b.name) || a.id.localeCompare(b.id));

function reindex(dashboards: DashboardSummary[], container: ContainerKey, ids: string[]) {
  const positions = new Map(ids.map((id, index) => [id, index]));
  return dashboards.map((dashboard) => {
    const sortOrder = positions.get(dashboard.id);
    return sortOrder === undefined ? dashboard : { ...dashboard, sidebarFolderId: folderId(container), sidebarSortOrder: sortOrder };
  });
}

/** Dashboard navigation shared by every client; the host supplies routing. */
export function DashboardSidebar({ activeDashboardId, onNavigate, footerControl }: { activeDashboardId?: string; onNavigate: (dashboardId: string) => void; footerControl?: React.ReactNode }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [sidebarEditMode, setSidebarEditMode] = React.useState(false);
  const dashboards = useQuery({ queryKey: dashboardsQueryKey, queryFn: ({ signal }) => api.getDashboards(signal) });
  const folders = useQuery({ queryKey: dashboardFoldersQueryKey, queryFn: ({ signal }) => api.getDashboardFolders(signal) });
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));

  const changePin = useMutation({
    mutationFn: ({ dashboard, pinned }: { dashboard: DashboardSummary; pinned: boolean }) => pinned ? api.pinDashboard(dashboard.id) : api.unpinDashboard(dashboard.id),
    onMutate: async ({ dashboard, pinned }) => {
      await queryClient.cancelQueries({ queryKey: dashboardsQueryKey });
      const previous = queryClient.getQueryData<DashboardSummary[]>(dashboardsQueryKey);
      queryClient.setQueryData<DashboardSummary[]>(dashboardsQueryKey, (current) => current?.map((item) => item.id === dashboard.id ? { ...item, pinned } : item));
      return { previous };
    },
    onError: (_error, _variables, context) => { queryClient.setQueryData(dashboardsQueryKey, context?.previous); toast.error("Could not update the dashboard pin."); },
    onSettled: () => queryClient.invalidateQueries({ queryKey: dashboardsQueryKey }),
  });

  const saveDashboardOrder = useMutation({
    mutationFn: async ({ source, target, movedId, sourceIds, targetIds }: { next: DashboardSummary[]; source: ContainerKey; target: ContainerKey; movedId: string; sourceIds: string[]; targetIds: string[] }) => {
      if (source === target) return api.reorderDashboardsInContext(folderId(target), targetIds);
      await api.updateDashboardSidebarPlacement(movedId, { folderId: folderId(target), sortOrder: targetIds.indexOf(movedId) });
      await Promise.all([api.reorderDashboardsInContext(folderId(source), sourceIds), api.reorderDashboardsInContext(folderId(target), targetIds)]);
    },
    onMutate: async ({ next }) => {
      await queryClient.cancelQueries({ queryKey: dashboardsQueryKey });
      const previous = queryClient.getQueryData<DashboardSummary[]>(dashboardsQueryKey);
      queryClient.setQueryData(dashboardsQueryKey, next);
      return { previous };
    },
    onError: (_error, _variables, context) => { queryClient.setQueryData(dashboardsQueryKey, context?.previous); toast.error("Could not save the dashboard order. The previous order was restored."); },
    onSettled: () => queryClient.invalidateQueries({ queryKey: dashboardsQueryKey }),
  });

  const saveFolderOrder = useMutation({
    mutationFn: ({ ids }: { ids: string[]; next: DashboardFolder[] }) => api.reorderDashboardFolders(ids),
    onMutate: async ({ next }) => {
      await queryClient.cancelQueries({ queryKey: dashboardFoldersQueryKey });
      const previous = queryClient.getQueryData<DashboardFolder[]>(dashboardFoldersQueryKey);
      queryClient.setQueryData(dashboardFoldersQueryKey, next);
      return { previous };
    },
    onError: (_error, _variables, context) => { queryClient.setQueryData(dashboardFoldersQueryKey, context?.previous); toast.error("Could not save the folder order. The previous order was restored."); },
    onSettled: () => queryClient.invalidateQueries({ queryKey: dashboardFoldersQueryKey }),
  });

  const visible = React.useMemo(() => (dashboards.data ?? []).filter((dashboard) => !dashboard.hidden), [dashboards.data]);
  const sortedFolders = React.useMemo(() => [...(folders.data ?? [])].sort((a, b) => a.sortOrder - b.sortOrder || a.name.localeCompare(b.name) || a.id.localeCompare(b.id)), [folders.data]);
  const knownFolders = React.useMemo(() => new Set(sortedFolders.map((folder) => folder.id)), [sortedFolders]);
  const normalized = React.useMemo(() => visible.map((dashboard) => dashboard.sidebarFolderId && !knownFolders.has(dashboard.sidebarFolderId) ? { ...dashboard, sidebarFolderId: null } : dashboard), [knownFolders, visible]);

  const moveDashboard = React.useCallback((dashboardId: string, requestedTarget: ContainerKey, targetIndex?: number) => {
    if (!dashboards.data) return;
    const dashboard = dashboards.data.find((item) => item.id === dashboardId);
    if (!dashboard) return;
    const source = dashboardContainer(dashboard);
    const target: ContainerKey = requestedTarget.startsWith("ungrouped:") ? dashboard.role === "owner" ? "ungrouped:owner" : "ungrouped:shared" : requestedTarget;
    const sourceItems = ordered(dashboards.data, source);
    const targetItems = source === target ? sourceItems : ordered(dashboards.data, target);
    const targetIds = targetItems.filter((item) => item.id !== dashboardId).map((item) => item.id);
    targetIds.splice(Math.max(0, Math.min(targetIndex ?? targetIds.length, targetIds.length)), 0, dashboardId);
    const sourceIds = sourceItems.filter((item) => item.id !== dashboardId).map((item) => item.id);
    saveDashboardOrder.mutate({ next: reindex(reindex(dashboards.data, source, sourceIds), target, targetIds), source, target, movedId: dashboardId, sourceIds, targetIds });
  }, [dashboards.data, saveDashboardOrder]);

  const moveFolder = React.useCallback((folderIdToMove: string, offset: -1 | 1) => {
    const from = sortedFolders.findIndex((folder) => folder.id === folderIdToMove);
    const to = from + offset;
    if (from < 0 || to < 0 || to >= sortedFolders.length) return;
    const next = arrayMove(sortedFolders, from, to).map((folder, sortOrder) => ({ ...folder, sortOrder }));
    saveFolderOrder.mutate({ next, ids: next.map((folder) => folder.id) });
  }, [saveFolderOrder, sortedFolders]);

  const handleDragEnd = React.useCallback(({ active, over }: DragEndEvent) => {
    if (!sidebarEditMode || !over || !dashboards.data || !folders.data) return;
    if (active.data.current?.type === "folder") {
      const activeId = String(active.data.current.folderId ?? "");
      const overId = String(
        over.data.current?.folderId ??
          (typeof over.data.current?.container === "string" &&
          over.data.current.container.startsWith("folder:")
            ? folderId(over.data.current.container as ContainerKey)
            : ""),
      );
      if (!activeId || !overId || activeId === overId) return;
      const current = [...folders.data].sort((a, b) => a.sortOrder - b.sortOrder);
      const from = current.findIndex((folder) => folder.id === activeId);
      const to = current.findIndex((folder) => folder.id === overId);
      if (from < 0 || to < 0) return;
      const next = arrayMove(current, from, to).map((folder, sortOrder) => ({ ...folder, sortOrder }));
      saveFolderOrder.mutate({ next, ids: next.map((folder) => folder.id) });
      return;
    }
    if (active.data.current?.type !== "dashboard") return;
    const id = String(active.data.current.dashboardId ?? "");
    const source = active.data.current.container as ContainerKey | undefined;
    const target = over.data.current?.container as ContainerKey | undefined;
    if (!id || !source || !target) return;
    const targetItems = ordered(dashboards.data, target).filter((dashboard) => dashboard.id !== id);
    const overId = over.data.current?.dashboardId as string | undefined;
    let index = overId ? targetItems.findIndex((dashboard) => dashboard.id === overId) : targetItems.length;
    if (source === target && overId) {
      const sourceItems = ordered(dashboards.data, source);
      const from = sourceItems.findIndex((dashboard) => dashboard.id === id);
      const to = sourceItems.findIndex((dashboard) => dashboard.id === overId);
      if (from < to) index += 1;
    }
    moveDashboard(id, target, index < 0 ? targetItems.length : index);
  }, [dashboards.data, folders.data, moveDashboard, saveFolderOrder, sidebarEditMode]);

  const error = dashboards.isError ? dashboards.error : folders.isError ? folders.error : null;
  return (
    <nav aria-label="Dashboard navigation" className="flex h-full flex-col gap-4 p-4">
      <div className="flex min-h-[var(--touch-target-size)] items-center justify-between gap-3 px-1">
        <span className="text-sm font-semibold">Dashboards</span>
        <Button
          type="button"
          variant={sidebarEditMode ? "secondary" : "ghost"}
          size="sm"
          aria-pressed={sidebarEditMode}
          onClick={() => setSidebarEditMode((current) => !current)}
        >
          {sidebarEditMode ? <Check data-icon="inline-start" aria-hidden="true" /> : <Pencil data-icon="inline-start" aria-hidden="true" />}
          {sidebarEditMode ? "Done" : "Edit"}
        </Button>
      </div>
      {sidebarEditMode ? <div className="flex gap-2"><DashboardCreateDialog onCreated={(dashboard) => onNavigate(dashboard.id)} trigger={<Button className="min-w-0 flex-1" size="sm"><Plus data-icon="inline-start" aria-hidden="true" />New dashboard</Button>} /><DashboardFolderDialog mode="create" trigger={<Button type="button" variant="outline" size="icon" aria-label="New folder"><FolderPlus aria-hidden="true" /></Button>} /></div> : null}
      {dashboards.isPending || folders.isPending ? <DashboardSidebarSkeleton /> : null}
      {error ? <Alert variant="destructive" className="px-3 py-2"><AlertCircle aria-hidden="true" /><AlertDescription>{describeConnectorError(error)}</AlertDescription></Alert> : null}
      {dashboards.isSuccess && folders.isSuccess ? (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <div className={cn("flex flex-col gap-3 rounded-lg border border-transparent p-1 transition-colors", sidebarEditMode && "border-border bg-muted/30")}>
            {normalized.some((dashboard) => dashboard.pinned) ? <DashboardSection title="Pinned">{normalized.filter((dashboard) => dashboard.pinned).map((dashboard) => <SidebarItem key={`pinned-${dashboard.id}`} dashboard={dashboard} container={dashboardContainer(dashboard)} folders={sortedFolders} siblings={ordered(normalized, dashboardContainer(dashboard))} active={dashboard.id === activeDashboardId} editMode={sidebarEditMode} draggable={false} pending={saveDashboardOrder.isPending} pinPending={changePin.isPending && changePin.variables.dashboard.id === dashboard.id} onNavigate={onNavigate} onMove={moveDashboard} onTogglePin={() => changePin.mutate({ dashboard, pinned: false })} />)}</DashboardSection> : null}
            <SortableContext items={sortedFolders.map((folder) => folderDragId(folder.id))} strategy={verticalListSortingStrategy}>
              {sortedFolders.map((folder, folderIndex) => {
                const container: ContainerKey = `folder:${folder.id}`;
                const members = ordered(normalized, container);
                return <FolderSection key={folder.id} folder={folder} count={members.length} editMode={sidebarEditMode} movePending={saveFolderOrder.isPending} canMoveUp={folderIndex > 0} canMoveDown={folderIndex < sortedFolders.length - 1} onMove={(offset) => moveFolder(folder.id, offset)}><DropZone container={container} dashboards={members} enabled={sidebarEditMode}>{members.length ? members.map((dashboard) => <SidebarItem key={dashboard.id} dashboard={dashboard} container={container} folders={sortedFolders} siblings={members} active={dashboard.id === activeDashboardId} editMode={sidebarEditMode} draggable={sidebarEditMode} pending={saveDashboardOrder.isPending} pinPending={changePin.isPending && changePin.variables.dashboard.id === dashboard.id} onNavigate={onNavigate} onMove={moveDashboard} onTogglePin={() => changePin.mutate({ dashboard, pinned: !dashboard.pinned })} />) : sidebarEditMode ? <p className="px-3 py-2 text-xs text-muted-foreground">Drop dashboards here.</p> : null}</DropZone></FolderSection>;
              })}
            </SortableContext>
            <UngroupedSection title="My Dashboards" container="ungrouped:owner" dashboards={ordered(normalized, "ungrouped:owner")} folders={sortedFolders} activeDashboardId={activeDashboardId} editMode={sidebarEditMode} pending={saveDashboardOrder.isPending} pinMutation={changePin} onNavigate={onNavigate} onMove={moveDashboard} empty="Use Edit to create your first dashboard." />
            {ordered(normalized, "ungrouped:shared").length ? <UngroupedSection title="Shared with me" container="ungrouped:shared" dashboards={ordered(normalized, "ungrouped:shared")} folders={sortedFolders} activeDashboardId={activeDashboardId} editMode={sidebarEditMode} pending={saveDashboardOrder.isPending} pinMutation={changePin} onNavigate={onNavigate} onMove={moveDashboard} /> : null}
          </div>
        </DndContext>
      ) : null}
      {footerControl === undefined ? null : <div className="mt-auto border-t pt-4">{footerControl}</div>}
    </nav>
  );
}

function UngroupedSection({ title, container, dashboards, folders, activeDashboardId, editMode, pending, pinMutation, onNavigate, onMove, empty }: { title: string; container: ContainerKey; dashboards: DashboardSummary[]; folders: DashboardFolder[]; activeDashboardId?: string; editMode: boolean; pending: boolean; pinMutation: { isPending: boolean; variables?: { dashboard: DashboardSummary; pinned: boolean }; mutate: (variables: { dashboard: DashboardSummary; pinned: boolean }) => void }; onNavigate: (id: string) => void; onMove: (id: string, target: ContainerKey, index?: number) => void; empty?: string }) {
  return <BuiltInFolderSection title={title} count={dashboards.length}><DropZone container={container} dashboards={dashboards} enabled={editMode}>{dashboards.length ? dashboards.map((dashboard) => <SidebarItem key={dashboard.id} dashboard={dashboard} container={container} folders={folders} siblings={dashboards} active={dashboard.id === activeDashboardId} editMode={editMode} draggable={editMode} pending={pending} pinPending={pinMutation.isPending && pinMutation.variables?.dashboard.id === dashboard.id} onNavigate={onNavigate} onMove={onMove} onTogglePin={() => pinMutation.mutate({ dashboard, pinned: !dashboard.pinned })} />) : <p className="px-3 py-2 text-xs text-muted-foreground">{empty}</p>}</DropZone></BuiltInFolderSection>;
}

/** Reusable create transaction for the sidebar and first-dashboard empty state. */
export function DashboardCreateDialog({ trigger, onCreated }: { trigger: React.ReactNode; onCreated: (dashboard: DashboardSummary) => void }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [open, setOpen] = React.useState(false);
  const [name, setName] = React.useState("");
  const mutation = useMutation({ mutationFn: () => api.createDashboard(name.trim()), onSuccess: async (dashboard) => { queryClient.setQueryData<DashboardSummary[]>(dashboardsQueryKey, (current) => current ? [...current, dashboard] : [dashboard]); await queryClient.invalidateQueries({ queryKey: dashboardsQueryKey }); setOpen(false); setName(""); onCreated(dashboard); } });
  return <NameDialog open={open} onOpenChange={(next) => { setOpen(next); if (!next) { mutation.reset(); setName(""); } }} trigger={trigger} title="New dashboard" description="Create a dashboard to arrange and control your services." label="Name" inputId="new-dashboard-name" value={name} placeholder="Operations" pending={mutation.isPending} error={mutation.error} action="Create dashboard" onChange={setName} onSubmit={() => mutation.mutate()} />;
}

function DashboardFolderDialog({ mode, folder, trigger, open: controlledOpen, onOpenChange }: { mode: "create" | "rename"; folder?: DashboardFolder; trigger?: React.ReactNode; open?: boolean; onOpenChange?: (open: boolean) => void }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [localOpen, setLocalOpen] = React.useState(false);
  const open = controlledOpen ?? localOpen;
  const setOpen = (next: boolean) => { setLocalOpen(next); onOpenChange?.(next); };
  const [name, setName] = React.useState(folder?.name ?? "");
  const [icon, setIcon] = React.useState<string | null>(folder?.icon ?? null);
  const mutation = useMutation({ mutationFn: () => mode === "create" ? api.createDashboardFolder(name.trim(), icon) : api.updateDashboardFolder(folder!.id, { name: name.trim(), icon }), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: dashboardFoldersQueryKey }); setOpen(false); if (mode === "create") { setName(""); setIcon(null); } } });
  return <NameDialog open={open} onOpenChange={(next) => { setOpen(next); if (next) { setName(folder?.name ?? ""); setIcon(folder?.icon ?? null); } else mutation.reset(); }} trigger={trigger} title={mode === "create" ? "New folder" : "Edit folder"} description="Folders organize only your personal dashboard sidebar." label="Name" inputId={`${mode}-folder-name-${folder?.id ?? "new"}`} value={name} placeholder="Infrastructure" pending={mutation.isPending} error={mutation.error} action={mode === "create" ? "Create folder" : "Save folder"} onChange={setName} onSubmit={() => mutation.mutate()} afterInput={<div className="flex flex-col gap-2"><Label>Icon</Label><GenericIconPicker value={icon} defaultIcon="lucide:folder" label="Folder icon" defaultLabel="Folder" onChange={setIcon} disabled={mutation.isPending} /></div>} />;
}

function NameDialog({ open, onOpenChange, trigger, title, description, label, inputId, value, placeholder, pending, error, action, onChange, onSubmit, afterInput }: { open: boolean; onOpenChange: (open: boolean) => void; trigger?: React.ReactNode; title: string; description: string; label: string; inputId: string; value: string; placeholder: string; pending: boolean; error: unknown; action: string; onChange: (value: string) => void; onSubmit: () => void; afterInput?: React.ReactNode }) {
  return <Dialog open={open} onOpenChange={onOpenChange}>{trigger ? <DialogTrigger asChild>{trigger}</DialogTrigger> : null}<DialogContent><DialogHeader><DialogTitle>{title}</DialogTitle><DialogDescription>{description}</DialogDescription></DialogHeader><form className="flex flex-col gap-4" onSubmit={(event) => { event.preventDefault(); if (value.trim()) onSubmit(); }}><div className="flex flex-col gap-2"><Label htmlFor={inputId}>{label}</Label><Input id={inputId} autoFocus value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder} /></div>{afterInput}{error ? <MutationError error={error} /> : null}<DialogFooter><Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={!value.trim() || pending}>{pending ? "Saving…" : action}</Button></DialogFooter></form></DialogContent></Dialog>;
}

function FolderSection({ folder, count, editMode, movePending, canMoveUp, canMoveDown, onMove, children }: { folder: DashboardFolder; count: number; editMode: boolean; movePending: boolean; canMoveUp: boolean; canMoveDown: boolean; onMove: (offset: -1 | 1) => void; children: React.ReactNode }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [open, setOpen] = React.useState(true);
  const [renameOpen, setRenameOpen] = React.useState(false);
  const [confirmDelete, setConfirmDelete] = React.useState(false);
  const sortable = useSortable({ id: folderDragId(folder.id), disabled: !editMode, data: { type: "folder", folderId: folder.id, container: `folder:${folder.id}` } });
  const deletion = useMutation({ mutationFn: () => api.deleteDashboardFolder(folder.id), onSuccess: async () => { setConfirmDelete(false); await Promise.all([queryClient.invalidateQueries({ queryKey: dashboardFoldersQueryKey }), queryClient.invalidateQueries({ queryKey: dashboardsQueryKey })]); } });
  return <section ref={sortable.setNodeRef} style={sortableStyle(sortable.transform, sortable.transition, sortable.isDragging)}><div className="flex items-center gap-1 px-1">{editMode ? <DragHandle label={`Reorder ${folder.name} folder`} attributes={sortable.attributes} listeners={sortable.listeners} /> : null}<FolderDisclosure name={folder.name} icon={folder.icon} count={count} open={open} onToggle={() => setOpen((current) => !current)} />{editMode ? <DropdownMenu><DropdownMenuTrigger asChild><Button type="button" variant="ghost" size="icon" className="shrink-0" aria-label={`Folder actions for ${folder.name}`}><MoreHorizontal aria-hidden="true" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuGroup><DropdownMenuItem onSelect={() => setRenameOpen(true)}>Edit folder</DropdownMenuItem><DropdownMenuItem disabled={!canMoveUp || movePending} onSelect={() => onMove(-1)}>Move up</DropdownMenuItem><DropdownMenuItem disabled={!canMoveDown || movePending} onSelect={() => onMove(1)}>Move down</DropdownMenuItem></DropdownMenuGroup><DropdownMenuSeparator /><DropdownMenuGroup><DropdownMenuItem className="text-destructive focus:text-destructive" onSelect={() => setConfirmDelete(true)}>Delete</DropdownMenuItem></DropdownMenuGroup></DropdownMenuContent></DropdownMenu> : null}</div><FolderContents open={open}>{children}</FolderContents><DashboardFolderDialog mode="rename" folder={folder} open={renameOpen} onOpenChange={setRenameOpen} /><AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>Delete “{folder.name}”?</AlertDialogTitle><AlertDialogDescription>The {count === 1 ? "dashboard" : `${count} dashboards`} in this folder will become ungrouped. No dashboards will be deleted.</AlertDialogDescription></AlertDialogHeader>{deletion.error ? <MutationError error={deletion.error} /> : null}<AlertDialogFooter><AlertDialogCancel disabled={deletion.isPending}>Cancel</AlertDialogCancel><AlertDialogAction className="bg-destructive text-destructive-foreground hover:bg-destructive/90" disabled={deletion.isPending} onClick={(event) => { event.preventDefault(); deletion.mutate(); }}>{deletion.isPending ? "Deleting…" : "Delete folder"}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog></section>;
}

function BuiltInFolderSection({ title, count, children }: { title: string; count: number; children: React.ReactNode }) {
  const [open, setOpen] = React.useState(true);
  return <section><div className="flex items-center gap-1 px-1"><FolderDisclosure name={title} icon={null} count={count} open={open} onToggle={() => setOpen((current) => !current)} /></div><FolderContents open={open}>{children}</FolderContents></section>;
}

function FolderDisclosure({ name, icon, count, open, onToggle }: { name: string; icon: string | null; count: number; open: boolean; onToggle: () => void }) {
  return <Button type="button" variant="ghost" size="sm" className="min-w-0 flex-1 justify-start gap-2 px-2 text-muted-foreground" aria-expanded={open} onClick={onToggle}><ChevronDown aria-hidden="true" className={cn("shrink-0 transition-transform [transition-duration:var(--motion-fast)]", !open && "-rotate-90")} /><ConnectorIcon typeIcon="lucide:folder" iconOverride={icon} size={18} /><span className="truncate">{name}</span><span className="ml-auto text-xs tabular-nums">{count}</span></Button>;
}

function FolderContents({ open, children }: { open: boolean; children: React.ReactNode }) {
  return <div className="motion-collapse-grid" data-state={open ? "open" : "closed"}><div className="min-h-0 overflow-hidden"><div className="mt-1 flex flex-col gap-2 px-1">{children}</div></div></div>;
}

function DropZone({ container, dashboards, enabled, children }: { container: ContainerKey; dashboards: DashboardSummary[]; enabled: boolean; children: React.ReactNode }) {
  const drop = useDroppable({ id: `dashboard-container:${container}`, disabled: !enabled, data: { type: "container", container } });
  return <SortableContext items={dashboards.map((dashboard) => dashboardDragId(dashboard.id))} strategy={verticalListSortingStrategy}><div ref={drop.setNodeRef} className={cn("min-h-2 rounded-md", drop.isOver && "bg-accent/40")}>{children}</div></SortableContext>;
}

function DashboardSection({ title, children }: { title: string; children: React.ReactNode }) {
  const [open, setOpen] = React.useState(true);
  return <section><Button type="button" variant="ghost" size="sm" className="w-full justify-between px-2 text-muted-foreground" aria-expanded={open} onClick={() => setOpen((current) => !current)}>{title}<ChevronDown aria-hidden="true" className={cn("transition-transform [transition-duration:var(--motion-fast)]", !open && "-rotate-90")} /></Button><div className="motion-collapse-grid" data-state={open ? "open" : "closed"}><div className="min-h-0 overflow-hidden"><div className="mt-1 flex flex-col gap-1">{children}</div></div></div></section>;
}

function SidebarItem({ dashboard, container, folders, siblings, active, editMode, draggable, pending, pinPending, onNavigate, onMove, onTogglePin }: { dashboard: DashboardSummary; container: ContainerKey; folders: DashboardFolder[]; siblings: DashboardSummary[]; active: boolean; editMode: boolean; draggable: boolean; pending: boolean; pinPending: boolean; onNavigate: (id: string) => void; onMove: (id: string, target: ContainerKey, index?: number) => void; onTogglePin: () => void }) {
  const sortable = useSortable({ id: draggable ? dashboardDragId(dashboard.id) : `pinned-dashboard:${dashboard.id}`, disabled: !draggable, data: { type: "dashboard", dashboardId: dashboard.id, container } });
  const index = siblings.findIndex((item) => item.id === dashboard.id);
  return <div ref={sortable.setNodeRef} style={sortableStyle(sortable.transform, sortable.transition, sortable.isDragging)} className={cn("group flex min-w-0 items-center gap-2 rounded-md px-1 py-1", active && "bg-muted text-foreground")}>{draggable ? <DragHandle label={`Reorder ${dashboard.name}`} attributes={sortable.attributes} listeners={sortable.listeners} /> : null}<DashboardNavigationButton dashboard={dashboard} active={active} onNavigate={onNavigate} /><DropdownMenu><DropdownMenuTrigger asChild><Button type="button" variant="ghost" size="icon" className="ml-1 shrink-0" aria-label={`Dashboard actions for ${dashboard.name}`}><MoreHorizontal aria-hidden="true" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuLabel>{editMode ? "Organize dashboard" : "Dashboard"}</DropdownMenuLabel><DropdownMenuGroup><DropdownMenuItem disabled={pinPending} onSelect={onTogglePin}>{dashboard.pinned ? "Unpin" : "Pin"}</DropdownMenuItem></DropdownMenuGroup>{editMode ? <><DropdownMenuSeparator /><DropdownMenuGroup><DropdownMenuSub><DropdownMenuSubTrigger>Move to folder</DropdownMenuSubTrigger><DropdownMenuSubContent><DropdownMenuGroup><DropdownMenuItem disabled={dashboard.sidebarFolderId === null || pending} onSelect={() => onMove(dashboard.id, dashboard.role === "owner" ? "ungrouped:owner" : "ungrouped:shared")}>Ungrouped</DropdownMenuItem>{folders.map((folder) => <DropdownMenuItem key={folder.id} disabled={dashboard.sidebarFolderId === folder.id || pending} onSelect={() => onMove(dashboard.id, `folder:${folder.id}`)}>{folder.name}</DropdownMenuItem>)}</DropdownMenuGroup></DropdownMenuSubContent></DropdownMenuSub><DropdownMenuItem disabled={index <= 0 || pending} onSelect={() => onMove(dashboard.id, container, index - 1)}>Move up</DropdownMenuItem><DropdownMenuItem disabled={index < 0 || index >= siblings.length - 1 || pending} onSelect={() => onMove(dashboard.id, container, index + 1)}>Move down</DropdownMenuItem></DropdownMenuGroup></> : null}</DropdownMenuContent></DropdownMenu></div>;
}

function DashboardNavigationButton({ dashboard, active, onNavigate }: { dashboard: DashboardSummary; active: boolean; onNavigate: (id: string) => void }) {
  const nameRef = React.useRef<HTMLSpanElement>(null);
  const longPressTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressed = React.useRef(false);
  const [truncated, setTruncated] = React.useState(false);
  const [tooltipOpen, setTooltipOpen] = React.useState(false);

  React.useLayoutEffect(() => {
    const name = nameRef.current;
    if (!name) return;
    const measure = () => setTruncated(name.scrollWidth > name.clientWidth);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(name);
    return () => observer.disconnect();
  }, [dashboard.name]);

  React.useEffect(() => () => {
    if (longPressTimer.current) clearTimeout(longPressTimer.current);
  }, []);

  const clearLongPress = () => {
    if (longPressTimer.current) clearTimeout(longPressTimer.current);
    longPressTimer.current = null;
  };
  const finishLongPress = () => {
    clearLongPress();
    if (longPressed.current) setTimeout(() => setTooltipOpen(false), 1400);
  };

  return (
    <Tooltip open={truncated && tooltipOpen} onOpenChange={(open) => setTooltipOpen(truncated && open)}>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          className="h-auto min-h-[var(--touch-target-size)] min-w-0 flex-1 justify-start gap-3 px-3 py-2.5"
          aria-current={active ? "page" : undefined}
          onPointerDown={(event) => {
            if (event.pointerType !== "touch" || !truncated) return;
            clearLongPress();
            longPressed.current = false;
            longPressTimer.current = setTimeout(() => {
              longPressed.current = true;
              setTooltipOpen(true);
            }, 600);
          }}
          onPointerUp={finishLongPress}
          onPointerCancel={finishLongPress}
          onPointerLeave={finishLongPress}
          onContextMenu={(event) => {
            if (truncated) event.preventDefault();
          }}
          onClick={(event) => {
            if (longPressed.current) {
              event.preventDefault();
              longPressed.current = false;
              return;
            }
            onNavigate(dashboard.id);
          }}
        >
          <span ref={nameRef} className="min-w-0 flex-1 truncate text-left">{dashboard.name}</span>
          {dashboard.role !== "owner" ? <Badge variant="outline" className="shrink-0 px-1.5 py-0 text-[0.625rem]">{dashboard.role === "editor" ? "Editor" : "Viewer"}</Badge> : null}
        </Button>
      </TooltipTrigger>
      {truncated ? <TooltipContent side="right">{dashboard.name}</TooltipContent> : null}
    </Tooltip>
  );
}

function DragHandle({ label, attributes, listeners }: { label: string; attributes: ReturnType<typeof useSortable>["attributes"]; listeners: ReturnType<typeof useSortable>["listeners"] }) {
  return <Button type="button" variant="ghost" size="icon" className="shrink-0 cursor-grab touch-none active:cursor-grabbing" aria-label={label} {...attributes} {...listeners}><GripVertical aria-hidden="true" /></Button>;
}

function sortableStyle(transform: { x: number; y: number; scaleX: number; scaleY: number } | null, transition: string | undefined, dragging: boolean): React.CSSProperties {
  return { transform: transform ? `translate3d(${transform.x}px, ${transform.y}px, 0) scaleX(${transform.scaleX}) scaleY(${transform.scaleY})` : undefined, transition, opacity: dragging ? 0.55 : undefined };
}
function MutationError({ error }: { error: unknown }) { return <Alert variant="destructive"><AlertCircle aria-hidden="true" /><AlertDescription>{describeConnectorError(error)}</AlertDescription></Alert>; }
function DashboardSidebarSkeleton() { return <div className="flex flex-col gap-3" aria-label="Loading dashboards"><Skeleton className="h-8 w-full" /><Skeleton className="h-8 w-5/6" /><Skeleton className="h-8 w-full" /></div>; }
