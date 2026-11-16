import "@ai-interview/ui/styles.css";
import "./admin.css";
import { applyTheme, readTheme, ToastProvider } from "@ai-interview/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { App } from "./App";
import { createQueryClient } from "./api/session";

applyTheme(readTheme(), false);

const queryClient = createQueryClient();

const root = document.getElementById("root");
if (!root) throw new Error("页面里没有 #root");

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <BrowserRouter basename={import.meta.env.BASE_URL.replace(/\/$/, "")}>
          <App />
        </BrowserRouter>
      </ToastProvider>
    </QueryClientProvider>
  </StrictMode>,
);
