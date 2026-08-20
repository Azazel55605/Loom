import * as React from "react";

import type { ApiClient } from "@loom/ui-kit/lib/api";

const ApiContext = React.createContext<ApiClient | null>(null);

export function ApiClientProvider({
  client,
  children,
}: {
  client: ApiClient;
  children: React.ReactNode;
}) {
  return <ApiContext.Provider value={client}>{children}</ApiContext.Provider>;
}

export function useApiClient(): ApiClient {
  const client = React.useContext(ApiContext);
  if (client === null) {
    throw new Error("useApiClient must be used within an AuthProvider");
  }
  return client;
}
