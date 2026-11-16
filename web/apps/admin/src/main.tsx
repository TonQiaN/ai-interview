import "@ai-interview/ui/styles.css";
import "./admin.css";
import { applyTheme, readTheme, ToastProvider } from "@ai-interview/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { App } from "./App";
import { createQueryClient } from "./api/session";

applyTheme(readTheme(), false);

const queryClient = createQueryClient();

// 页面还是写在 App 里的 <Routes>。用这种路由是为了表单填到一半要离开时能先拦下来问一句（useLeaveGuard）
const router = createBrowserRouter([{ path: "*", element: <App /> }], {
  basename: import.meta.env.BASE_URL.replace(/\/$/, "") || "/",
});

const root = document.getElementById("root");
if (!root) throw new Error("页面里没有 #root");

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <RouterProvider router={router} />
      </ToastProvider>
    </QueryClientProvider>
  </StrictMode>,
);
