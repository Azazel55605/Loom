/** The close information shared by browser and native socket implementations. */
export type TransportSocketCloseEvent = {
  code: number;
  reason: string;
};

/** One connected text WebSocket, independent of the platform that owns it. */
export interface TransportSocket {
  send(data: string): Promise<void>;
  close(): Promise<void>;
  onOpen(callback: () => void): void;
  onMessage(callback: (data: string) => void): void;
  onClose(callback: (event: TransportSocketCloseEvent) => void): void;
  onError(callback: (error: unknown) => void): void;
}

/** Platform-owned WebSocket creation, mirroring the injected HTTP transport. */
export interface WebSocketTransport {
  connect(url: string): Promise<TransportSocket>;
}
