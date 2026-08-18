import React from "react";
import ReactDOM from "react-dom/client";

import App from "@/App";
import { AccentThemeProvider } from "@/components/AccentThemeProvider";
import "@/index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AccentThemeProvider>
      <App />
    </AccentThemeProvider>
  </React.StrictMode>,
);
