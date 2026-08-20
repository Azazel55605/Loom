import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import App from "@/App";
import { AccentThemeProvider } from "@loom/ui-kit/components/AccentThemeProvider";
import "@loom/ui-kit/styles.css";

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
        <App queryClient={queryClient} />
      </AccentThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
