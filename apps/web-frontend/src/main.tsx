import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";

import App from "@/App";
import { AccentThemeProvider } from "@loom/ui-kit/components/AccentThemeProvider";
import { Toaster } from "@loom/ui-kit/components/ui/sonner";
import { AuthProvider } from "@loom/ui-kit/lib/auth-context";
import "@loom/ui-kit/styles.css";
import { webBaseUrl, webBaseUrlProvider } from "@/adapters/webBaseUrlProvider";
import { webTokenStorage } from "@/adapters/webTokenStorage";
import { webWebSocketTransport } from "@/adapters/webWebSocketTransport";

/**
 * One client for the whole app.
 *
 * Retries are off by default: most failures here are a backend that is not
 * running, and retrying three times only delays showing the user that fact.
 * Queries that genuinely benefit from a retry opt in individually.
 */
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: false,
    },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <AccentThemeProvider>
        <BrowserRouter>
          <AuthProvider
            baseUrlProvider={webBaseUrlProvider}
            bootstrapBaseUrl={webBaseUrl}
            tokenStorage={webTokenStorage}
            webSocketTransport={webWebSocketTransport}
          >
            <App />
            <Toaster />
          </AuthProvider>
        </BrowserRouter>
      </AccentThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
