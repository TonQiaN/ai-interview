import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// 候选人面试页。部署在 /i/ 下，开发时把 /api 转给本机后端。
export default defineConfig({
  base: "/i/",
  plugins: [react()],
  build: {
    // 后端从 web/dist 托管两个前端的构建产物
    outDir: "../../dist/candidate",
    emptyOutDir: true,
  },
  server: {
    port: 5174,
    proxy: {
      "/api": { target: "http://127.0.0.1:8080", ws: true },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
