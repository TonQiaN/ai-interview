import "@ai-interview/ui/styles.css";
import "./app.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

const root = document.getElementById("root");
if (!root) throw new Error("页面缺少 #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
