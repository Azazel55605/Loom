import * as React from "react";

import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@loom/ui-kit/components/ui/table";
import type { AuditLogEntry } from "@loom/ui-kit/lib/api";

const COLLAPSED_MESSAGE_LENGTH = 96;

/** Shared action-history table used by one connector and the global audit log. */
export function AuditLogTable({
  entries,
  showInstance = false,
  onOpenInstance,
}: {
  entries: AuditLogEntry[];
  showInstance?: boolean;
  onOpenInstance?: (instanceId: string) => void;
}) {
  const [expandedMessages, setExpandedMessages] = React.useState<Set<string>>(
    () => new Set(),
  );

  const toggleMessage = React.useCallback((id: string) => {
    setExpandedMessages((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  return (
    <div className="max-w-full overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            {showInstance ? <TableHead>Instance</TableHead> : null}
            <TableHead>Action</TableHead>
            <TableHead>Target</TableHead>
            <TableHead>Invoked by</TableHead>
            <TableHead>When</TableHead>
            <TableHead>Outcome</TableHead>
            <TableHead>Result</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {entries.map((entry) => {
            const message = entry.resultMessage ?? "—";
            const expandable = message.length > COLLAPSED_MESSAGE_LENGTH;
            const expanded = expandedMessages.has(entry.id);
            const instanceId = entry.instanceId;
            return (
              <TableRow key={entry.id}>
                {showInstance ? (
                  <TableCell>
                    {instanceId && onOpenInstance ? (
                      <Button
                        type="button"
                        variant="link"
                        className="h-auto justify-start p-0 text-left"
                        onClick={() => onOpenInstance(instanceId)}
                      >
                        {entry.instanceName ?? entry.instanceId}
                      </Button>
                    ) : (
                      <span className="font-medium">
                        {entry.instanceName ?? entry.instanceId ?? "Unknown instance"}
                      </span>
                    )}
                    {entry.connectorType ? (
                      <span className="block text-xs text-muted-foreground">
                        {entry.connectorType}
                      </span>
                    ) : null}
                  </TableCell>
                ) : null}
                <TableCell className="font-mono text-xs">{entry.actionId}</TableCell>
                <TableCell>{entry.targetId ?? "—"}</TableCell>
                <TableCell>{actorLabel(entry)}</TableCell>
                <TableCell className="whitespace-nowrap">
                  <time dateTime={entry.invokedAt}>{formatTimestamp(entry.invokedAt)}</time>
                  <span className="block text-xs text-muted-foreground">
                    {durationLabel(entry)}
                  </span>
                </TableCell>
                <TableCell>
                  <OutcomeBadge success={entry.success} />
                </TableCell>
                <TableCell className="max-w-xs">
                  {expandable ? (
                    <Button
                      type="button"
                      variant="link"
                      className="h-auto max-w-full justify-start whitespace-normal p-0 text-left"
                      aria-expanded={expanded}
                      onClick={() => toggleMessage(entry.id)}
                    >
                      {expanded
                        ? message
                        : `${message.slice(0, COLLAPSED_MESSAGE_LENGTH).trimEnd()}…`}
                    </Button>
                  ) : (
                    <span className="break-words">{message}</span>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

function OutcomeBadge({ success }: { success: boolean | null }) {
  if (success === null) return <Badge variant="pending">Pending</Badge>;
  return success ? (
    <Badge variant="healthy">Success</Badge>
  ) : (
    <Badge variant="destructive">Failed</Badge>
  );
}

function actorLabel(entry: AuditLogEntry): string {
  if (entry.invokedBy.system) return "Loom system";
  return entry.invokedBy.username ?? "Unknown user";
}

function formatTimestamp(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function durationLabel(entry: AuditLogEntry): string {
  if (entry.completedAt === null) return "Still running";
  const started = new Date(entry.invokedAt).getTime();
  const completed = new Date(entry.completedAt).getTime();
  if (!Number.isFinite(started) || !Number.isFinite(completed)) return "Completed";
  const elapsed = Math.max(0, completed - started);
  if (elapsed < 1_000) return `${elapsed} ms`;
  return `${(elapsed / 1_000).toFixed(elapsed < 10_000 ? 1 : 0)} s`;
}
