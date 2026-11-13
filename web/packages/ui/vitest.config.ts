import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    // 默认会把样式文件当成空的；对比度测试要读 tokens.css 的原文
    css: { include: /tokens\.css/ },
  },
});
