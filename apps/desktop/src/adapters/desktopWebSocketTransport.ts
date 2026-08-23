import TauriWebSocket, {
  type CloseFrame,
  type Message,
} from "@tauri-apps/plugin-websocket";

import type {
  TransportSocket,
  TransportSocketCloseEvent,
  WebSocketTransport,
} from "@loom/ui-kit/lib/websocket-transport";

class DesktopTransportSocket implements TransportSocket {
  private readonly openListeners = new Set<() => void>();
  private readonly messageListeners = new Set<(data: string) => void>();
  private readonly closeListeners = new Set<
    (event: TransportSocketCloseEvent) => void
  >();
  private readonly errorListeners = new Set<(error: unknown) => void>();
  private closeEvent: TransportSocketCloseEvent | null = null;
  private error: unknown = null;

  constructor(private readonly socket: TauriWebSocket) {
    socket.addListener((message) => this.handleMessage(message));
  }

  async send(data: string): Promise<void> {
    await this.socket.send(data);
  }

  async close(): Promise<void> {
    if (this.closeEvent !== null) return;
    this.emitClose({ code: 1000, reason: "Disconnected by client" });
    await this.socket.disconnect();
  }

  onOpen(callback: () => void): void {
    this.openListeners.add(callback);
    // The native plugin resolves `connect()` only after the handshake. Defer
    // this notification so ConnectorStatusSocket can register every handler
    // before it sends the initial subscription.
    queueMicrotask(() => {
      if (this.closeEvent === null && this.openListeners.delete(callback)) callback();
    });
  }

  onMessage(callback: (data: string) => void): void {
    this.messageListeners.add(callback);
  }

  onClose(callback: (event: TransportSocketCloseEvent) => void): void {
    if (this.closeEvent !== null) {
      const event = this.closeEvent;
      queueMicrotask(() => callback(event));
      return;
    }
    this.closeListeners.add(callback);
  }

  onError(callback: (error: unknown) => void): void {
    if (this.error !== null) {
      const error = this.error;
      queueMicrotask(() => callback(error));
      return;
    }
    this.errorListeners.add(callback);
  }

  private handleMessage(message: Message | unknown): void {
    // The Rust plugin serializes a stream read error through the same channel,
    // although its published TypeScript union currently describes only frames.
    if (typeof message === "string") {
      this.emitError(new Error(message));
      return;
    }
    if (typeof message !== "object" || message === null || !("type" in message)) {
      this.emitError(new Error("The native WebSocket transport returned an unknown event."));
      return;
    }

    const frame = message as Message;
    if (frame.type === "Text") {
      for (const listener of this.messageListeners) listener(frame.data);
      return;
    }
    if (frame.type === "Close") {
      this.emitClose(closeEvent(frame.data));
    }
  }

  private emitClose(event: TransportSocketCloseEvent): void {
    if (this.closeEvent !== null) return;
    this.closeEvent = event;
    for (const listener of this.closeListeners) listener(event);
    this.closeListeners.clear();
    this.openListeners.clear();
  }

  private emitError(error: unknown): void {
    this.error = error;
    for (const listener of this.errorListeners) listener(error);
  }
}

function closeEvent(frame: CloseFrame | null): TransportSocketCloseEvent {
  return frame ?? { code: 1005, reason: "Connection closed without a close frame" };
}

/**
 * Native Desktop WebSocket transport for plain `ws:` and normally validated
 * `wss:` connections.
 *
 * `@tauri-apps/plugin-websocket` 2.4.2 exposes buffer/header options to JS but
 * no per-connection invalid-certificate policy. Its Rust builder accepts only
 * a process-wide startup TLS connector, which cannot follow Loom's runtime
 * per-server setting. Consequently the HTTP certificate exception does not yet
 * extend to self-signed WSS; keep this adapter native so that support can be
 * added here when the upstream plugin gains that option.
 */
export const desktopWebSocketTransport: WebSocketTransport = {
  async connect(url) {
    const socket = await TauriWebSocket.connect(url);
    return new DesktopTransportSocket(socket);
  },
};

export const desktopInvalidCertificateWebSocketNote =
  "Initial loads and actions use this certificate exception, but live status updates over WSS still require a certificate trusted by the operating system.";
