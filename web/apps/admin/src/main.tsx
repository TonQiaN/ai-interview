import "@ai-interview/ui/styles.css";
import "./admin.css";
import { applyTheme, readTheme, ToastProvider } from "@ai-interview/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { App } from "./App";
import { ApiError } from "./api/http";

applyTheme(readTheme(), false);

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: false,
      // 4xx 重试也不会变，只对网络错误和 5xx 再试一次
      retry: (count, error) =>
        count < 1 && !(error instanceof ApiError && error.status >= 400 && error.status < 500),
    },
  },
});

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
