import * as React from "react";
import { useQueries } from "@tanstack/react-query";

import { ConnectorIcon } from "@loom/ui-kit/components/ConnectorIcon";
import type {
  ConnectorInstanceDetail,
  ConnectorStatus,
  ScreensaverDataPoint,
} from "@loom/ui-kit/lib/api";
import { useApiClient, useConnectorStatusSocket } from "@loom/ui-kit/lib/api-context";
import { useAppearance } from "@loom/ui-kit/components/AccentThemeProvider";
import { statusDetailsForTarget } from "@loom/ui-kit/lib/connector-details";
import {
  formatNumericReadingText,
  formatReading,
} from "@loom/ui-kit/widgets/types";

const ROTATION_INTERVAL_MS = 8_000;

type LiveStatus = {
  status: ConnectorStatus | null;
};

export function ScreensaverView({
  config,
  onDismiss,
}: {
  config: ScreensaverDataPoint[];
  onDismiss: () => void;
}) {
  const api = useApiClient();
  const socket = useConnectorStatusSocket();
  const { effectiveReduceMotion } = useAppearance();
  const [now, setNow] = React.useState(() => new Date());
  const [index, setIndex] = React.useState(0);
  const [previousIndex, setPreviousIndex] = React.useState<number | null>(null);
  const lastIndex = React.useRef(0);
  const fadeTimer = React.useRef<number | null>(null);
  const [live, setLive] = React.useState<Record<string, LiveStatus>>({});
  const instanceIds = React.useMemo(
    () => [...new Set(config.map((entry) => entry.connectorInstanceId))],
    [config],
  );
  const detailQueries = useQueries({
    queries: instanceIds.map((instanceId) => ({
      queryKey: ["connector-instance", instanceId],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        api.getConnectorInstance(instanceId, signal),
    })),
  });

  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  React.useEffect(() => {
    if (config.length < 2) return;
    const timer = window.setInterval(
      () => setIndex((current) => (current + 1) % config.length),
      ROTATION_INTERVAL_MS,
    );
    return () => window.clearInterval(timer);
  }, [config.length]);

  React.useEffect(() => {
    setIndex((current) => Math.min(current, Math.max(config.length - 1, 0)));
  }, [config.length]);

  React.useEffect(() => {
    if (lastIndex.current === index) return;
    if (fadeTimer.current !== null) window.clearTimeout(fadeTimer.current);
    if (effectiveReduceMotion) {
      setPreviousIndex(null);
    } else {
      setPreviousIndex(lastIndex.current);
      fadeTimer.current = window.setTimeout(() => {
        setPreviousIndex(null);
        fadeTimer.current = null;
      }, 420);
    }
    lastIndex.current = index;
    return () => {
      if (fadeTimer.current !== null) window.clearTimeout(fadeTimer.current);
    };
  }, [effectiveReduceMotion, index]);

  React.useEffect(() => {
    if (instanceIds.length === 0) return;
    return socket.subscribe(instanceIds, (update) => {
      setLive((current) => ({
        ...current,
        [update.instanceId]: { status: update.status },
      }));
    });
  }, [instanceIds, socket]);

  const details = React.useMemo(() => {
    const byId = new Map<string, ConnectorInstanceDetail>();
    detailQueries.forEach((query, queryIndex) => {
      if (query.data !== undefined) byId.set(instanceIds[queryIndex], query.data);
    });
    return byId;
  }, [detailQueries, instanceIds]);

  function ambientReading(readingIndex: number, phase: "current" | "previous") {
    const selection = config[readingIndex];
    if (selection === undefined) return null;
    const detail = details.get(selection.connectorInstanceId);
    const descriptor = detail?.dataPoints.find(
      (point) => point.id === selection.dataPointId && point.targetId === selection.targetId,
    );
    const status = live[selection.connectorInstanceId]?.status ?? detail?.status ?? null;
    const reading = statusDetailsForTarget(status?.details, selection.targetId)[
      selection.dataPointId
    ];
    const formatted =
      typeof reading === "number"
        ? formatNumericReadingText(reading, descriptor?.unit)
        : formatReading(reading);

    return (
      <section
        key={`${phase}:${selection.connectorInstanceId}:${selection.targetId ?? ""}:${selection.dataPointId}:${readingIndex}`}
        className={
          effectiveReduceMotion
            ? "mobile-kiosk-reading"
            : `mobile-kiosk-reading mobile-kiosk-reading-${phase === "current" ? "in" : "out"}`
        }
        aria-live={phase === "current" ? "polite" : "off"}
      >
        <div className="flex items-center justify-center gap-3 text-muted-foreground">
          <ConnectorIcon
            typeIcon={detail?.metadata.icon ?? null}
            iconOverride={detail?.iconOverride ?? null}
            size={28}
          />
          <span className="text-base font-medium">{detail?.name ?? "Connector"}</span>
        </div>
        <p className="mt-5 text-sm uppercase tracking-[0.2em] text-muted-foreground">
          {descriptor?.label ?? selection.dataPointId}
        </p>
        <p className="mt-2 break-words text-4xl font-semibold tracking-tight sm:text-5xl">
          {formatted}
        </p>
      </section>
    );
  }

  return (
    <main
      className="mobile-kiosk-screensaver"
      onPointerDown={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDismiss();
      }}
      onTouchStart={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onDismiss();
      }}
    >
      <div className="flex flex-col items-center text-center">
        <time className="mobile-kiosk-clock" dateTime={now.toISOString()}>
          {new Intl.DateTimeFormat(undefined, {
            hour: "2-digit",
            minute: "2-digit",
          }).format(now)}
        </time>
        <p className="mt-2 text-lg text-muted-foreground">
          {new Intl.DateTimeFormat(undefined, {
            weekday: "long",
            year: "numeric",
            month: "long",
            day: "numeric",
          }).format(now)}
        </p>
      </div>

      {config.length > 0 ? (
        <div className="mobile-kiosk-reading-stage">
          {previousIndex === null ? null : ambientReading(previousIndex, "previous")}
          {ambientReading(index, "current")}
        </div>
      ) : null}
    </main>
  );
}
