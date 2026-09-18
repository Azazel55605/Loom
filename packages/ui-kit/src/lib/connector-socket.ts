import type {
  ApiClient,
  ConnectorError,
  ConnectorStatus,
  PendingOperation,
} from "@loom/ui-kit/lib/api";
import type {
  TransportSocket,
  WebSocketTransport,
} from "@loom/ui-kit/lib/websocket-transport";
import {
  INITIAL_RETRY_DELAY_MS as INITIAL_RECONNECT_DELAY_MS,
  nextRetryDelayMs,
} from "@loom/ui-kit/lib/session-failure";


export type ConnectorStatusUpdate = {
  type: "status";
  instanceId: string;
  status: ConnectorStatus | null;
  statusError?: ConnectorError;
  /** A disruptive action in flight. Takes visual precedence over `status`. */
  pendingOperation: PendingOperation | null;
  /** Why this instance is Down, probed from the network beneath it. */
  diagnosis: string | null;
};

export type NetworkAdvisoryUpdate = {
  type: "networkAdvisory";
  active: boolean;
  affectedHostCount: number;
};

type StatusListener = (update: ConnectorStatusUpdate) => void;
type NetworkAdvisoryListener = (update: NetworkAdvisoryUpdate) => void;

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

function isNetworkAdvisoryUpdate(value: unknown): value is NetworkAdvisoryUpdate {
  if (typeof value !== "object" || value === null) return false;
  const update = value as Partial<NetworkAdvisoryUpdate>;
  return (
    update.type === "networkAdvisory" &&
    typeof update.active === "boolean" &&
    typeof update.affectedHostCount === "number"
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
  private readonly advisoryListeners = new Set<NetworkAdvisoryListener>();
  private lastAdvisory: NetworkAdvisoryUpdate | null = null;
  private socket: TransportSocket | null = null;
  private socketOpen = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelayMs = INITIAL_RECONNECT_DELAY_MS;
  private generation = 0;
  private connecting = false;
  private disposed = false;
  private accessToken: string | null;
  private readonly unsubscribeFromTokens: () => void;

  constructor(
    private readonly api: ApiClient,
    private readonly transport: WebSocketTransport,
  ) {
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

    if (newlyActive.length > 0 && this.socketOpen) {
      this.send("subscribe", newlyActive);
    }
    this.ensureConnected();

    return () => this.removeListener(instanceIds, listener);
  }

  /** Listen to the retained system-wide network advisory. Unlike status
   * subscriptions this is intentionally unscoped: every signed-in shell needs
   * to know when several distinct hosts fail together. */
  subscribeNetworkAdvisory(listener: NetworkAdvisoryListener): () => void {
    this.advisoryListeners.add(listener);
    if (this.lastAdvisory !== null) {
      queueMicrotask(() => {
        if (this.advisoryListeners.has(listener) && this.lastAdvisory !== null) {
          listener(this.lastAdvisory);
        }
      });
    }
    this.ensureConnected();
    return () => {
      this.advisoryListeners.delete(listener);
      this.afterUnsubscribe([]);
    };
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
    this.advisoryListeners.clear();
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
    if (inactive.length > 0 && this.socketOpen) {
      this.send("unsubscribe", inactive);
    }
    if (!this.hasListeners()) {
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
      !this.hasListeners() ||
      this.accessToken === null
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
          !this.hasListeners() ||
          this.accessToken === null
        ) {
          return;
        }

        return this.transport.connect(websocketUrl(baseUrl, this.accessToken));
      })
      .then((socket) => {
        if (socket === undefined) return;
        if (
          this.disposed ||
          generation !== this.generation ||
          !this.hasListeners() ||
          this.accessToken === null
        ) {
          void socket.close().catch(() => undefined);
          return;
        }

        this.socket = socket;
        socket.onOpen(() => {
          if (socket !== this.socket) return;
          this.socketOpen = true;
          this.reconnectDelayMs = INITIAL_RECONNECT_DELAY_MS;
          this.send("subscribe", [...this.listeners.keys()]);
        });
        socket.onMessage((data) => {
          try {
            const message: unknown = JSON.parse(data);
            if (isStatusUpdate(message)) {
              for (const listener of this.listeners.get(message.instanceId) ?? []) {
                listener(message);
              }
            } else if (isNetworkAdvisoryUpdate(message)) {
              this.lastAdvisory = message;
              for (const listener of this.advisoryListeners) {
                listener(message);
              }
            }
          } catch {
            // Unknown or malformed server messages are ignored; a later valid
            // status update still keeps the connection useful.
          }
        });
        socket.onClose(() => {
          if (socket !== this.socket) return;
          this.socket = null;
          this.socketOpen = false;
          this.scheduleReconnect();
        });
        socket.onError(() => this.failSocket(socket));
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
    this.socketOpen = false;
    if (socket !== null) {
      void socket.close().catch(() => undefined);
    }
  }

  private failSocket(socket: TransportSocket): void {
    if (socket !== this.socket) return;
    this.socket = null;
    this.socketOpen = false;
    void socket.close().catch(() => undefined);
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    if (
      this.disposed ||
      !this.hasListeners() ||
      this.accessToken === null ||
      this.reconnectTimer !== null
    ) {
      return;
    }
    const delay = this.reconnectDelayMs;
    this.reconnectDelayMs = nextRetryDelayMs(this.reconnectDelayMs);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.ensureConnected();
    }, delay);
  }

  private cancelReconnect(): void {
    if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
  }

  private hasListeners(): boolean {
    return this.listeners.size > 0 || this.advisoryListeners.size > 0;
  }

  private send(type: "subscribe" | "unsubscribe", instanceIds: readonly string[]): void {
    const socket = this.socket;
    if (instanceIds.length === 0 || socket === null || !this.socketOpen) return;
    void socket
      .send(JSON.stringify({ type, instanceIds }))
      .catch(() => this.failSocket(socket));
  }
}
