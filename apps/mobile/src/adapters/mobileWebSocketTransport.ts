import type {
  TransportSocket,
  WebSocketTransport,
} from "@loom/ui-kit/lib/websocket-transport";

class MobileTransportSocket implements TransportSocket {
  constructor(private readonly socket: WebSocket) {}

  async send(data: string): Promise<void> {
    this.socket.send(data);
  }

  async close(): Promise<void> {
    this.socket.close();
  }

  onOpen(callback: () => void): void {
    this.socket.addEventListener("open", callback, { once: true });
  }

  onMessage(callback: (data: string) => void): void {
    this.socket.addEventListener("message", (event) => {
      if (typeof event.data === "string") callback(event.data);
    });
  }

  onClose(callback: (event: { code: number; reason: string }) => void): void {
    this.socket.addEventListener("close", (event) => {
      callback({ code: event.code, reason: event.reason });
    });
  }

  onError(callback: (error: unknown) => void): void {
    this.socket.addEventListener("error", callback);
  }
}

/** Mobile remains explicit until it adopts a native socket transport. */
export const mobileWebSocketTransport: WebSocketTransport = {
  async connect(url) {
    return new MobileTransportSocket(new WebSocket(url));
  },
};
