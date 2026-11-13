import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// HR 管理后台。部署在 /admin/ 下，开发时把 /api 转给本机后端。
export default defineConfig({
  base: "/admin/",
  plugins: [react()],
  build: {
    // 后端从 web/dist 托管两个前端的构建产物
    outDir: "../../dist/admin",
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    // 演示数据读的是 crates/interview-report/samples 里的合成样例，在 web/ 之外
    fs: { allow: ["../../.."] },
    proxy: {
      "/api": { target: "http://127.0.0.1:8080" },
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
