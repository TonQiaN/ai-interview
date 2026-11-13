import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// 组件展示页：pnpm --filter @ai-interview/ui dev
export default defineConfig({
  root: new URL(".", import.meta.url).pathname,
  plugins: [react()],
  server: { port: 5175 },
  build: { outDir: "../../../dist/ui-gallery", emptyOutDir: true },
});
