import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import "./index.css";

// Live-ish data: poll on an interval and on focus, matching the TUI's 30s refresh but
// snappier for a foreground browser tab.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchInterval: 15000,
      refetchOnWindowFocus: true,
      staleTime: 5000,
      retry: 1,
    },
  },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
