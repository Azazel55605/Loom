import * as React from "react";
import { Check, Ellipsis, Plus, Server } from "lucide-react";

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
import { Button } from "@loom/ui-kit/components/ui/button";
import { AddServerFlow, type ServerConnection } from "@loom/ui-kit/components/ConnectToServer";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@loom/ui-kit/components/ui/dropdown-menu";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import type { HttpTransport } from "@loom/ui-kit/lib/api";
import type {
  ServerProfile,
  ServerProfileManager,
} from "@loom/ui-kit/lib/server-profile";
import { cn } from "@loom/ui-kit/lib/utils";

type ServerSwitcherContextValue = {
  activeProfile: ServerProfile | null;
  openSwitcher: () => void;
};

const ServerSwitcherContext = React.createContext<ServerSwitcherContextValue | null>(null);

function profileHost(baseUrl: string): string {
  try {
    return new URL(baseUrl).host;
  } catch {
    return baseUrl;
  }
}

export type ServerSwitcherProviderProps = {
  manager: ServerProfileManager;
  supportsInvalidCertificates?: boolean;
  invalidCertificateNote?: string;
  getHttpTransport?: (allowInvalidCertificates: boolean) => HttpTransport;
  /** Persists platform-owned connection details, clears server-derived UI state,
   * and lets the existing auth/bootstrap tree remount for the active profile. */
  onActiveProfileChanged: (connection?: ServerConnection) => Promise<void>;
  children: React.ReactNode;
};

/** Owns one switcher surface for the whole app, including boot-error recovery. */
export function ServerSwitcherProvider({
  manager,
  supportsInvalidCertificates = false,
  invalidCertificateNote,
  getHttpTransport,
  onActiveProfileChanged,
  children,
}: ServerSwitcherProviderProps) {
  const [open, setOpen] = React.useState(false);
  const [profiles, setProfiles] = React.useState<ServerProfile[]>([]);
  const [activeProfileId, setActiveProfileId] = React.useState<string | null>(null);

  const refreshProfiles = React.useCallback(async () => {
    const [nextProfiles, nextActiveProfileId] = await Promise.all([
      manager.listProfiles(),
      manager.getActiveProfileId(),
    ]);
    setProfiles(nextProfiles);
    setActiveProfileId(nextActiveProfileId);
  }, [manager]);

  React.useEffect(() => {
    void refreshProfiles().catch(() => undefined);
  }, [refreshProfiles]);

  const value = React.useMemo<ServerSwitcherContextValue>(
    () => ({
      activeProfile:
        profiles.find((profile) => profile.id === activeProfileId) ?? null,
      openSwitcher: () => setOpen(true),
    }),
    [activeProfileId, profiles],
  );

  return (
    <ServerSwitcherContext.Provider value={value}>
      {children}
      <ServerSwitcher
        open={open}
        onOpenChange={setOpen}
        profiles={profiles}
        activeProfileId={activeProfileId}
        manager={manager}
        supportsInvalidCertificates={supportsInvalidCertificates}
        invalidCertificateNote={invalidCertificateNote}
        getHttpTransport={getHttpTransport}
        refreshProfiles={refreshProfiles}
        onActiveProfileChanged={onActiveProfileChanged}
      />
    </ServerSwitcherContext.Provider>
  );
}

export function useServerSwitcher(): ServerSwitcherContextValue {
  const context = React.useContext(ServerSwitcherContext);
  if (context === null) {
    throw new Error("useServerSwitcher must be used within ServerSwitcherProvider");
  }
  return context;
}

/** Header/settings access point; the switching UI itself remains mounted once. */
export function ServerSwitcherTrigger({
  compact = false,
  manageLabel = false,
}: {
  compact?: boolean;
  manageLabel?: boolean;
}) {
  const { activeProfile, openSwitcher } = useServerSwitcher();
  const label = manageLabel
    ? "Manage servers"
    : activeProfile?.label ?? "Choose server";

  return (
    <Button
      type="button"
      variant={manageLabel ? "outline" : "ghost"}
      size="sm"
      className={cn(
        "min-w-0",
        compact ? "max-w-36 px-2" : "max-w-56",
      )}
      title={
        activeProfile === null
          ? label
          : `${activeProfile.label} — ${profileHost(activeProfile.baseUrl)}`
      }
      onClick={openSwitcher}
    >
      <Server aria-hidden="true" />
      <span className="truncate">{label}</span>
    </Button>
  );
}

type ServerSwitcherProps = Omit<ServerSwitcherProviderProps, "children"> & {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  profiles: ServerProfile[];
  activeProfileId: string | null;
  refreshProfiles: () => Promise<void>;
};

/** Device-local profile selection and management, shared by Desktop and Mobile. */
export function ServerSwitcher({
  open,
  onOpenChange,
  profiles,
  activeProfileId,
  manager,
  supportsInvalidCertificates,
  invalidCertificateNote,
  getHttpTransport,
  refreshProfiles,
  onActiveProfileChanged,
}: ServerSwitcherProps) {
  const [addOpen, setAddOpen] = React.useState(false);
  const [renameProfile, setRenameProfile] = React.useState<ServerProfile | null>(null);
  const [removeProfile, setRemoveProfile] = React.useState<ServerProfile | null>(null);
  const [renameDraft, setRenameDraft] = React.useState("");
  const [pendingProfileId, setPendingProfileId] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) {
      void refreshProfiles().catch((refreshError: unknown) => {
        setError(
          refreshError instanceof Error
            ? refreshError.message
            : "The server profiles could not be loaded.",
        );
      });
    }
  }, [open, refreshProfiles]);

  const run = React.useCallback(async (operation: () => Promise<void>) => {
    setError(null);
    try {
      await operation();
    } catch (operationError) {
      setError(
        operationError instanceof Error
          ? operationError.message
          : "The server profiles could not be updated.",
      );
    }
  }, []);

  const selectProfile = async (profile: ServerProfile) => {
    if (profile.id === activeProfileId) {
      onOpenChange(false);
      return;
    }
    setPendingProfileId(profile.id);
    await run(async () => {
      await manager.setActiveProfileId(profile.id);
      await refreshProfiles();
      onOpenChange(false);
      await onActiveProfileChanged();
    });
    setPendingProfileId(null);
  };

  const addProfile = async (connection: ServerConnection, label?: string) => {
    const created = await manager.addProfile(label ?? profileHost(connection.baseUrl), connection.baseUrl);
    try {
      await manager.setActiveProfileId(created.id);
      await onActiveProfileChanged(connection);
      await refreshProfiles();
      setAddOpen(false);
      onOpenChange(false);
    } catch (addError) {
      await manager.removeProfile(created.id).catch(() => undefined);
      await refreshProfiles();
      throw addError;
    }
  };

  const confirmRename = async () => {
    if (renameProfile === null || renameDraft.trim() === "") return;
    await run(async () => {
      await manager.renameProfile(renameProfile.id, renameDraft);
      await refreshProfiles();
      setRenameProfile(null);
    });
  };

  const confirmRemove = async () => {
    if (removeProfile === null) return;
    const removedActiveProfile = removeProfile.id === activeProfileId;
    await run(async () => {
      await manager.removeProfile(removeProfile.id);
      await refreshProfiles();
      setRemoveProfile(null);
      if (removedActiveProfile) {
        onOpenChange(false);
        await onActiveProfileChanged();
      }
    });
  };

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) {
            setAddOpen(false);
            setRenameProfile(null);
          }
          onOpenChange(nextOpen);
        }}
      >
        <DialogContent
          className={cn(
            "max-h-[calc(100vh-2rem)] w-[calc(100vw-2rem)] overflow-y-auto",
            addOpen ? "max-w-lg" : "max-w-md",
          )}
        >
          {addOpen ? (
            <>
              <DialogHeader>
                <DialogTitle>Add server</DialogTitle>
                <DialogDescription>
                  Loom verifies the server before saving this profile.
                </DialogDescription>
              </DialogHeader>
              <AddServerFlow
                embedded
                supportsInvalidCertificates={supportsInvalidCertificates}
                invalidCertificateNote={invalidCertificateNote}
                getHttpTransport={getHttpTransport}
                onConnected={addProfile}
              />
              <Button type="button" variant="ghost" onClick={() => setAddOpen(false)}>
                Back to servers
              </Button>
            </>
          ) : renameProfile !== null ? (
            <>
              <DialogHeader>
                <DialogTitle>Rename server</DialogTitle>
                <DialogDescription>
                  Change this profile&apos;s device-local label.
                </DialogDescription>
              </DialogHeader>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="rename-server-profile">Label</Label>
                <Input
                  id="rename-server-profile"
                  value={renameDraft}
                  autoFocus
                  onChange={(event) => setRenameDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void confirmRename();
                  }}
                />
              </div>
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setRenameProfile(null)}
                >
                  Cancel
                </Button>
                <Button
                  type="button"
                  disabled={renameDraft.trim() === ""}
                  onClick={() => void confirmRename()}
                >
                  Rename
                </Button>
              </DialogFooter>
            </>
          ) : (
            <>
              <DialogHeader>
                <DialogTitle>Servers</DialogTitle>
                <DialogDescription>
                  Switch between Loom servers. Each keeps its own session on this
                  device.
                </DialogDescription>
              </DialogHeader>

              {error === null ? null : (
                <Alert variant="destructive">
                  <AlertTitle>Could not update servers</AlertTitle>
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              )}

              <div className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto">
                {profiles.map((profile) => {
                  const active = profile.id === activeProfileId;
                  return (
                    <div
                      key={profile.id}
                      className={cn(
                        "flex min-w-0 items-center gap-1 rounded-md border p-1",
                        active && "border-primary/50 bg-accent",
                      )}
                    >
                      <Button
                        type="button"
                        variant="ghost"
                        className="h-auto min-h-[var(--touch-target-size)] min-w-0 flex-1 justify-start px-2 py-2 text-left"
                        disabled={pendingProfileId !== null}
                        onClick={() => void selectProfile(profile)}
                      >
                        <span className="flex min-w-0 flex-1 flex-col items-start">
                          <span className="w-full truncate font-medium">
                            {profile.label}
                          </span>
                          <span className="w-full truncate text-xs font-normal text-muted-foreground">
                            {profileHost(profile.baseUrl)}
                          </span>
                        </span>
                        {active ? <Check aria-label="Active server" /> : null}
                      </Button>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={`Manage ${profile.label}`}
                            disabled={pendingProfileId !== null}
                          >
                            <Ellipsis aria-hidden="true" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuGroup>
                            <DropdownMenuItem
                              onSelect={() => {
                                setRenameDraft(profile.label);
                                setRenameProfile(profile);
                              }}
                            >
                              Rename
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              onSelect={() => setRemoveProfile(profile)}
                            >
                              Remove
                            </DropdownMenuItem>
                          </DropdownMenuGroup>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  );
                })}
              </div>

              <Button
                type="button"
                variant="outline"
                onClick={() => setAddOpen(true)}
              >
                <Plus aria-hidden="true" />
                Add server
              </Button>
            </>
          )}
        </DialogContent>
      </Dialog>

      <AlertDialog
        open={removeProfile !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) setRemoveProfile(null);
        }}
      >
        <AlertDialogContent className="w-[calc(100vw-2rem)] max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this server?</AlertDialogTitle>
            <AlertDialogDescription>
              {removeProfile?.label ?? "This server"} and its stored session will be
              removed from this device. Nothing on the Loom server is deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={() => void confirmRemove()}>
              Remove server
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
