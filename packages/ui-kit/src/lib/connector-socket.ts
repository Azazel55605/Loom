import type { ApiClient, ConnectorError, ConnectorStatus } from "@loom/ui-kit/lib/api";

const INITIAL_RECONNECT_DELAY_MS = 1_000;
const MAX_RECONNECT_DELAY_MS = 30_000;

export type ConnectorStatusUpdate = {
  type: "status";
  instanceId: string;
  status: ConnectorStatus | null;
  statusError?: ConnectorError;
};

type StatusListener = (update: ConnectorStatusUpdate) => void;

function websocketUrl(baseUrl: string, accessToken: string): string {
  const fallback = typeof window === "undefined" ? "http://localhost/" : window.location.href;
  const url = new URL(baseUrl, fallback);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = `${url.pathname.replace(/\/$/, "")}/ws`;
  url.search = "";
  url.hash = "";
  url.searchParams.set("token", accessToken);
  return url.toString();
}

function isStatusUpdate(value: unknown): value is ConnectorStatusUpdate {
  if (typeof value !== "object" || value === null) return false;
  const update = value as Partial<ConnectorStatusUpdate>;
  return (
    update.type === "status" &&
    typeof update.instanceId === "string" &&
    (update.status === null || typeof update.status === "object")
  );
}

/**
 * One reconnecting connector-status socket for an authenticated API client.
 *
 * Subscriptions are reference-counted by callback. Reconnects re-send the
 * complete active id set, and an access-token rotation replaces the socket so
 * the next handshake never keeps using an expired credential.
 */
export class ConnectorStatusSocket {
  private readonly listeners = new Map<string, Set<StatusListener>>();
  private socket: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelayMs = INITIAL_RECONNECT_DELAY_MS;
  private generation = 0;
  private connecting = false;
  private disposed = false;
  private accessToken: string | null;
  private readonly unsubscribeFromTokens: () => void;

  constructor(private readonly api: ApiClient) {
    this.accessToken = api.tokenStore.getAccessToken();
    this.unsubscribeFromTokens = api.tokenStore.subscribe(() => {
      const next = api.tokenStore.getAccessToken();
      if (next === this.accessToken) return;
      this.accessToken = next;
      this.restart();
    });
  }

  subscribe(instanceIds: readonly string[], listener: StatusListener): () => void {
    const newlyActive: string[] = [];
    for (const id of new Set(instanceIds)) {
      let callbacks = this.listeners.get(id);
      if (callbacks === undefined) {
        callbacks = new Set();
        this.listeners.set(id, callbacks);
        newlyActive.push(id);
      }
      callbacks.add(listener);
    }

    if (newlyActive.length > 0 && this.socket?.readyState === WebSocket.OPEN) {
      this.send("subscribe", newlyActive);
    }
    this.ensureConnected();

    return () => this.removeListener(instanceIds, listener);
  }

  /** Remove every listener for these instances. Usually the cleanup returned
   * by `subscribe` is the more precise choice. */
  unsubscribe(instanceIds: readonly string[]): void {
    const inactive: string[] = [];
    for (const id of new Set(instanceIds)) {
      if (this.listeners.delete(id)) inactive.push(id);
    }
    this.afterUnsubscribe(inactive);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.unsubscribeFromTokens();
    this.listeners.clear();
    this.cancelReconnect();
    this.disconnect();
  }

  private removeListener(instanceIds: readonly string[], listener: StatusListener): void {
    const inactive: string[] = [];
    for (const id of new Set(instanceIds)) {
      const callbacks = this.listeners.get(id);
      if (callbacks === undefined) continue;
      callbacks.delete(listener);
      if (callbacks.size === 0) {
        this.listeners.delete(id);
        inactive.push(id);
      }
    }
    this.afterUnsubscribe(inactive);
  }

  private afterUnsubscribe(inactive: string[]): void {
    if (inactive.length > 0 && this.socket?.readyState === WebSocket.OPEN) {
      this.send("unsubscribe", inactive);
    }
    if (this.listeners.size === 0) {
      this.cancelReconnect();
      this.disconnect();
    }
  }

  private ensureConnected(): void {
    if (
      this.disposed ||
      this.connecting ||
      this.socket !== null ||
      this.reconnectTimer !== null ||
      this.listeners.size === 0 ||
      this.accessToken === null ||
      typeof WebSocket === "undefined"
    ) {
      return;
    }

    this.connecting = true;
    const generation = this.generation;
    void this.api
      .getBaseUrl()
      .then((baseUrl) => {
        if (
          this.disposed ||
          generation !== this.generation ||
          this.listeners.size === 0 ||
          this.accessToken === null
        ) {
          return;
        }

        const socket = new WebSocket(websocketUrl(baseUrl, this.accessToken));
        this.socket = socket;
        socket.onopen = () => {
          if (socket !== this.socket) return;
          this.reconnectDelayMs = INITIAL_RECONNECT_DELAY_MS;
          this.send("subscribe", [...this.listeners.keys()]);
        };
        socket.onmessage = (event) => {
          if (typeof event.data !== "string") return;
          try {
            const update: unknown = JSON.parse(event.data);
            if (!isStatusUpdate(update)) return;
            for (const listener of this.listeners.get(update.instanceId) ?? []) {
              listener(update);
            }
          } catch {
            // Unknown or malformed server messages are ignored; a later valid
            // status update still keeps the connection useful.
          }
        };
        socket.onclose = () => {
          if (socket !== this.socket) return;
          this.socket = null;
          this.scheduleReconnect();
        };
        socket.onerror = () => socket.close();
      })
      .catch(() => this.scheduleReconnect())
      .finally(() => {
        if (generation === this.generation) this.connecting = false;
      });
  }

  private restart(): void {
    this.generation += 1;
    this.connecting = false;
    this.cancelReconnect();
    this.disconnect();
    this.reconnectDelayMs = INITIAL_RECONNECT_DELAY_MS;
    this.ensureConnected();
  }

  private disconnect(): void {
    const socket = this.socket;
    this.socket = null;
    if (socket !== null) {
      socket.onclose = null;
      socket.close();
    }
  }

  private scheduleReconnect(): void {
    if (
      this.disposed ||
      this.listeners.size === 0 ||
      this.accessToken === null ||
      this.reconnectTimer !== null
    ) {
      return;
    }
    const delay = this.reconnectDelayMs;
    this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, MAX_RECONNECT_DELAY_MS);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.ensureConnected();
    }, delay);
  }

  private cancelReconnect(): void {
    if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
  }

  private send(type: "subscribe" | "unsubscribe", instanceIds: readonly string[]): void {
    if (instanceIds.length === 0 || this.socket?.readyState !== WebSocket.OPEN) return;
    this.socket.send(JSON.stringify({ type, instanceIds }));
  }
}
