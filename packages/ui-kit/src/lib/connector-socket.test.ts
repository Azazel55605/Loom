import { describe, expect, it, vi } from "vitest";

import type { ApiClient } from "@loom/ui-kit/lib/api";
import { ConnectorStatusSocket } from "@loom/ui-kit/lib/connector-socket";
import type {
  TransportSocket,
  TransportSocketCloseEvent,
  WebSocketTransport,
} from "@loom/ui-kit/lib/websocket-transport";

class FakeSocket implements TransportSocket {
  readonly sent: string[] = [];
  private openListener: (() => void) | null = null;
  private messageListener: ((data: string) => void) | null = null;
  private closeListener: ((event: TransportSocketCloseEvent) => void) | null = null;

  async send(data: string) {
    this.sent.push(data);
  }

  async close() {}

  onOpen(callback: () => void) {
    this.openListener = callback;
  }

  onMessage(callback: (data: string) => void) {
    this.messageListener = callback;
  }

  onClose(callback: (event: TransportSocketCloseEvent) => void) {
    this.closeListener = callback;
  }

  onError(_callback: (error: unknown) => void) {}

  open() {
    this.openListener?.();
  }

  message(value: unknown) {
    this.messageListener?.(JSON.stringify(value));
  }

  closeFromServer() {
    this.closeListener?.({ code: 1000, reason: "test" });
  }
}

function apiClient(): ApiClient {
  return {
    getBaseUrl: vi.fn(async () => "https://loom.example/api"),
    tokenStore: {
      getAccessToken: () => "access-token",
      subscribe: () => () => undefined,
    },
  } as unknown as ApiClient;
}

async function connectedClient() {
  const socket = new FakeSocket();
  const transport: WebSocketTransport = { connect: vi.fn(async () => socket) };
  const client = new ConnectorStatusSocket(apiClient(), transport);
  const listener = vi.fn();
  client.subscribe(["instance-1"], listener);
  await vi.waitFor(() => expect(transport.connect).toHaveBeenCalledOnce());
  await Promise.resolve();
  socket.open();
  return { client, socket, listener };
}

describe("ConnectorStatusSocket", () => {
  it("delivers a recovered status after a down status without suppressing it", async () => {
    const { client, socket, listener } = await connectedClient();
    socket.message({
      type: "status",
      instanceId: "instance-1",
      status: { health: "down", targetHealth: {}, details: {}, lastChecked: "first" },
      pendingOperation: null,
      diagnosis: "unreachable",
    });
    socket.message({
      type: "status",
      instanceId: "instance-1",
      status: { health: "healthy", targetHealth: {}, details: {}, lastChecked: "second" },
      pendingOperation: null,
      diagnosis: null,
    });

    expect(listener).toHaveBeenCalledTimes(2);
    expect(listener.mock.calls[1]?.[0].status.health).toBe("healthy");
    client.dispose();
  });

  it("delivers network advisories without an instance subscription", async () => {
    const socket = new FakeSocket();
    const transport: WebSocketTransport = { connect: vi.fn(async () => socket) };
    const client = new ConnectorStatusSocket(apiClient(), transport);
    const listener = vi.fn();
    client.subscribeNetworkAdvisory(listener);
    await vi.waitFor(() => expect(transport.connect).toHaveBeenCalledOnce());
    await Promise.resolve();
    socket.open();
    socket.message({ type: "networkAdvisory", active: true, affectedHostCount: 3 });

    expect(listener).toHaveBeenCalledWith({
      type: "networkAdvisory",
      active: true,
      affectedHostCount: 3,
    });
    client.dispose();
  });
});
