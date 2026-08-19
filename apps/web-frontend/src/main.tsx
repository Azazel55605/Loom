import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";

import App from "@/App";
import { AccentThemeProvider } from "@/components/AccentThemeProvider";
import { Toaster } from "@/components/ui/sonner";
import { AuthProvider } from "@/lib/auth-context";
import "@/index.css";

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
          <AuthProvider>
            <App />
            <Toaster />
          </AuthProvider>
        </BrowserRouter>
      </AccentThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
