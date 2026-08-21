import * as React from "react";

import type { ApiClient } from "@loom/ui-kit/lib/api";
import type { ConnectorStatusSocket } from "@loom/ui-kit/lib/connector-socket";

const ApiContext = React.createContext<ApiClient | null>(null);
const ConnectorSocketContext = React.createContext<ConnectorStatusSocket | null>(null);

export function ApiClientProvider({
  client,
  connectorSocket,
  children,
}: {
  client: ApiClient;
  connectorSocket: ConnectorStatusSocket;
  children: React.ReactNode;
}) {
  return (
    <ApiContext.Provider value={client}>
      <ConnectorSocketContext.Provider value={connectorSocket}>
        {children}
      </ConnectorSocketContext.Provider>
    </ApiContext.Provider>
  );
}

export function useConnectorStatusSocket(): ConnectorStatusSocket {
  const socket = React.useContext(ConnectorSocketContext);
  if (socket === null) {
    throw new Error("useConnectorStatusSocket must be used within an AuthProvider");
  }
  return socket;
}

export function useApiClient(): ApiClient {
  const client = React.useContext(ApiContext);
  if (client === null) {
    throw new Error("useApiClient must be used within an AuthProvider");
  }
  return client;
}
