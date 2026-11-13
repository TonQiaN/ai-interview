import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

const root = document.getElementById("root");
if (!root) throw new Error("页面缺少 #root 节点");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
