import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown,
  ListMusic,
  Music2,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Square,
  Volume2,
} from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@loom/ui-kit/components/ui/collapsible";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { Slider } from "@loom/ui-kit/components/ui/slider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@loom/ui-kit/components/ui/tooltip";
import type { MediaDuration, MediaItem, MediaTransportCommand, PlaybackState } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";
import { usePlaybackPosition } from "@loom/ui-kit/lib/use-playback-position";
import { BrowsableListGrid } from "@loom/ui-kit/widgets/BrowsableListGrid";
import { ImageDisplayWidget } from "@loom/ui-kit/widgets/ImageDisplay";

const PLAYBACK_REFRESH_MS = 12_000;

function durationSeconds(duration: MediaDuration | null | undefined): number | null {
  if (duration === null || duration === undefined) return null;
  return Math.max(0, duration[0] + duration[1] / 1_000_000_000);
}

function formatTime(seconds: number): string {
  const safe = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(safe / 60);
  return `${minutes}:${String(safe % 60).padStart(2, "0")}`;
}

type PlayerProps = {
  instanceId: string;
  targetId: string | null;
  showBrowser: boolean;
  supportsMediaSource: boolean;
  supportsMediaTarget: boolean;
  disabled?: boolean;
  unavailableReason?: string | null;
  expanded?: boolean;
  className?: string;
};

/** Composite now-playing, transport, browser, and queue widget. */
export function MediaPlayerWidget({
  instanceId,
  targetId,
  showBrowser,
  supportsMediaSource,
  supportsMediaTarget,
  disabled = false,
  unavailableReason,
  expanded = false,
  className,
}: PlayerProps) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const enabled = targetId !== null && supportsMediaTarget;
  const playbackKey = ["media-playback", instanceId, targetId] as const;
  const playback = useQuery({
    queryKey: playbackKey,
    queryFn: ({ signal }) => api.getPlaybackState(instanceId, targetId as string, signal),
    enabled,
    refetchInterval: PLAYBACK_REFRESH_MS,
  });
  const refresh = React.useCallback(async () => {
    await playback.refetch();
    await queryClient.invalidateQueries({ queryKey: ["media-queue", instanceId, targetId] });
  }, [instanceId, playback, queryClient, targetId]);
  const reportError = React.useCallback((error: unknown) => {
    toast.error("Media control failed", { description: describeConnectorError(error) });
  }, []);
  const transport = useMutation({
    mutationFn: (command: MediaTransportCommand) =>
      api.sendTransportCommand(instanceId, targetId as string, command),
    onSuccess: refresh,
    onError: reportError,
  });
  const seek = useMutation({
    mutationFn: (seconds: number) => api.seek(instanceId, targetId as string, Math.round(seconds)),
    onSuccess: refresh,
    onError: reportError,
  });
  const volume = useMutation({
    mutationFn: (percent: number) => api.setVolume(instanceId, targetId as string, Math.round(percent)),
    onSuccess: refresh,
    onError: reportError,
  });
  const play = useMutation({
    mutationFn: (item: MediaItem | { id: string }) =>
      api.playItem(instanceId, targetId as string, item.id),
    onSuccess: refresh,
    onError: reportError,
  });

  if (targetId === null) {
    return <MediaNotice className={className}>Choose a media-player target to use this widget.</MediaNotice>;
  }
  if (!supportsMediaTarget) {
    return <MediaNotice className={className}>This target does not provide media playback controls.</MediaNotice>;
  }
  if (playback.isPending) return <MediaPlayerSkeleton className={className} expanded={expanded} />;
  if (playback.isError) {
    return <MediaNotice className={className}>{describeConnectorError(playback.error)}</MediaNotice>;
  }

  const state = playback.data;
  const controlsDisabled = disabled || unavailableReason != null || transport.isPending;
  const controlReason = unavailableReason ?? (disabled ? "You do not have permission to control this connector." : null);
  return (
    <section className={cn("flex min-w-0 flex-col gap-4", className)}>
      <NowPlayingDisplay
        state={state}
        expanded={expanded}
        disabled={controlsDisabled || seek.isPending}
        disabledReason={controlReason}
        onSeek={(seconds) => seek.mutate(seconds)}
      />
      <TransportControls
        state={state}
        disabled={controlsDisabled}
        disabledReason={controlReason}
        volumePending={volume.isPending}
        onCommand={(command) => transport.mutate(command)}
        onVolume={(percent) => volume.mutate(percent)}
      />

      {showBrowser ? (
        supportsMediaSource ? (
          <div className="border-t pt-4">
            <BrowsableListGrid
              instanceId={instanceId}
              targetId={null}
              actionId={null}
              onExecute={async () => undefined}
              onLeafClick={(item) => play.mutateAsync(item)}
              disabled={disabled || unavailableReason != null || play.isPending}
            />
          </div>
        ) : (
          <MediaNotice>The media browser is unavailable because this connector has no media source.</MediaNotice>
        )
      ) : null}

      <QueueView instanceId={instanceId} targetId={targetId} />
    </section>
  );
}

/** On-demand view of the target's playback queue. */
export function QueueView({ instanceId, targetId }: { instanceId: string; targetId: string }) {
  const api = useApiClient();
  const [open, setOpen] = React.useState(false);
  const queue = useQuery({
    queryKey: ["media-queue", instanceId, targetId],
    queryFn: ({ signal }) => api.getQueue(instanceId, targetId, signal),
    enabled: open,
  });

  return (
      <Collapsible open={open} onOpenChange={setOpen} className="border-t pt-3">
        <CollapsibleTrigger asChild>
          <Button type="button" variant="ghost" className="w-full justify-between">
            <span className="flex items-center gap-2"><ListMusic className="size-4" />Queue</span>
            <ChevronDown className={cn("size-4 transition-transform", open && "rotate-180")} />
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-2">
          {queue.isPending ? <Skeleton className="h-24 w-full" /> : queue.isError ? (
            <p className="rounded-md border border-destructive/40 p-3 text-sm text-destructive">
              {describeConnectorError(queue.error)}
            </p>
          ) : queue.data?.items.length === 0 ? (
            <p className="p-3 text-sm text-muted-foreground">The queue is empty.</p>
          ) : (
            <ol className="max-h-64 space-y-1 overflow-y-auto">
              {queue.data?.items.map((item, index) => (
                <li key={`${item.id}-${index}`} className={cn(
                  "rounded-md px-3 py-2 text-sm",
                  queue.data.currentIndex === index && "bg-accent/15 text-foreground",
                )}>
                  <span className="block break-words font-medium">{item.title}</span>
                  {item.artist ? <span className="text-xs text-muted-foreground">{item.artist}</span> : null}
                </li>
              ))}
            </ol>
          )}
        </CollapsibleContent>
      </Collapsible>
  );
}

export function NowPlayingDisplay({
  state,
  expanded,
  disabled,
  disabledReason,
  onSeek,
}: {
  state: PlaybackState;
  expanded: boolean;
  disabled: boolean;
  disabledReason?: string | null;
  onSeek: (seconds: number) => void;
}) {
  const item = state.currentItem;
  const duration = durationSeconds(item?.duration);
  const position = usePlaybackPosition(state);
  const [dragValue, setDragValue] = React.useState<number | null>(null);
  const shownPosition = dragValue ?? position;

  return (
    <div className={cn("grid min-w-0 gap-4", expanded ? "sm:grid-cols-[minmax(12rem,18rem)_1fr]" : "sm:grid-cols-[9rem_1fr]")}>
      <ImageDisplayWidget
        label={item?.title ?? "Now playing"}
        value={item?.artworkRef}
        config={{ fit: "contain" }}
        expanded={expanded}
        fallbackIcon={<Music2 className="size-10" aria-hidden="true" />}
        showCaption={false}
      />
      <div className="flex min-w-0 flex-col justify-center gap-2">
        <div>
          <p className="break-words text-lg font-semibold">{item?.title ?? "Nothing playing"}</p>
          {item?.artist ? <p className="break-words text-sm text-muted-foreground">{item.artist}</p> : null}
          {item?.album ? <p className="break-words text-xs text-muted-foreground">{item.album}</p> : null}
        </div>
        {item && duration === null ? <Badge className="w-fit">LIVE</Badge> : null}
        {item && duration !== null ? (
          <div className="space-y-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="py-1">
                  <Slider
                    value={[Math.min(duration, shownPosition)]}
                    min={0}
                    max={Math.max(1, duration)}
                    step={1}
                    disabled={disabled}
                    aria-label="Playback position"
                    onValueChange={([value]) => setDragValue(value ?? 0)}
                    onValueCommit={([value]) => {
                      setDragValue(null);
                      if (value !== undefined) onSeek(value);
                    }}
                  />
                </div>
              </TooltipTrigger>
              <TooltipContent>{disabledReason ?? "Seek"}</TooltipContent>
            </Tooltip>
            <div className="flex justify-between text-xs tabular-nums text-muted-foreground">
              <span>{formatTime(shownPosition)}</span><span>{formatTime(duration)}</span>
            </div>
            {disabledReason ? <span className="sr-only">{disabledReason}</span> : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function TransportControls({
  state,
  disabled,
  disabledReason,
  volumePending,
  onCommand,
  onVolume,
}: {
  state: PlaybackState;
  disabled: boolean;
  disabledReason?: string | null;
  volumePending: boolean;
  onCommand: (command: MediaTransportCommand) => void;
  onVolume: (percent: number) => void;
}) {
  const [volume, setLocalVolume] = React.useState(state.volumePercent);
  React.useEffect(() => {
    if (!volumePending) setLocalVolume(state.volumePercent);
  }, [state.volumePercent, volumePending]);
  const reason = disabledReason ?? (disabled ? "Media controls are unavailable." : null);
  const playing = state.status === "playing";

  return (
    <div className="flex flex-wrap items-center justify-center gap-1">
      <Control label="Previous" reason={reason}><Button type="button" size="icon" variant="ghost" disabled={disabled} onClick={() => onCommand("skipPrevious")}><SkipBack /></Button></Control>
      <Control label={playing ? "Pause" : "Play"} reason={reason}><Button type="button" size="icon" disabled={disabled} onClick={() => onCommand(playing ? "pause" : "resume")}>{playing ? <Pause /> : <Play />}</Button></Control>
      <Control label="Stop" reason={reason}><Button type="button" size="icon" variant="ghost" disabled={disabled} onClick={() => onCommand("stop")}><Square /></Button></Control>
      <Control label="Next" reason={reason}><Button type="button" size="icon" variant="ghost" disabled={disabled} onClick={() => onCommand("skipNext")}><SkipForward /></Button></Control>
      <Control label={state.shuffle ? "Disable shuffle" : "Enable shuffle"} reason={reason}><Button type="button" size="icon" variant={state.shuffle ? "secondary" : "ghost"} disabled={disabled} onClick={() => onCommand("toggleShuffle")}><Shuffle /></Button></Control>
      <Control label={`Repeat: ${state.repeat}`} reason={reason}><Button type="button" className="gap-1 px-3" variant={state.repeat !== "off" ? "secondary" : "ghost"} disabled={disabled} onClick={() => onCommand("toggleRepeat")}>{state.repeat === "one" ? <Repeat1 /> : <Repeat />}<span className="text-xs capitalize">{state.repeat}</span></Button></Control>
      <Tooltip>
        <TooltipTrigger asChild>
          <div className="ml-2 flex min-w-[9rem] flex-1 items-center gap-2 sm:max-w-52">
            <Volume2 className="size-4 shrink-0 text-muted-foreground" />
            <Slider
              value={[volume]}
              min={0}
              max={100}
              step={1}
              disabled={disabled || volumePending}
              aria-label="Volume"
              onValueChange={([value]) => setLocalVolume(value ?? 0)}
              onValueCommit={([value]) => value !== undefined && onVolume(value)}
            />
            <span className="w-9 text-right text-xs tabular-nums text-muted-foreground">{volume}%</span>
          </div>
        </TooltipTrigger>
        <TooltipContent>{reason ?? "Volume"}</TooltipContent>
      </Tooltip>
    </div>
  );
}

function Control({ label, reason, children }: { label: string; reason: string | null; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild><span className="inline-flex" aria-label={label}>{children}</span></TooltipTrigger>
      <TooltipContent>{reason ?? label}</TooltipContent>
    </Tooltip>
  );
}

function MediaNotice({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("flex min-h-28 items-center justify-center rounded-lg border border-dashed p-4 text-center text-sm text-muted-foreground", className)}>{children}</div>;
}

export function MediaPlayerSkeleton({ className, expanded = false }: { className?: string; expanded?: boolean }) {
  return (
    <div className={cn("grid gap-4", expanded ? "sm:grid-cols-[18rem_1fr]" : "sm:grid-cols-[9rem_1fr]", className)}>
      <Skeleton className={cn("w-full rounded-lg", expanded ? "h-56" : "aspect-square min-h-32")} />
      <div className="space-y-3 py-3"><Skeleton className="h-6 w-2/3" /><Skeleton className="h-4 w-1/2" /><Skeleton className="h-4 w-full" /><Skeleton className="h-11 w-full" /></div>
    </div>
  );
}
