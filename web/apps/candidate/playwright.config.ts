import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { defineConfig } from "@playwright/test";
import { writeFakeMic } from "./e2e/fake-mic";

/**
 * 浏览器端到端测试：真的 Chromium，假的摄像头和麦克风，接口用 dev/mock-room-server.mjs。
 *
 *   pnpm e2e                 # 全部
 *   pnpm e2e -g 刷新         # 只跑名字里带「刷新」的
 *
 * 用到摄像头、麦克风、MediaRecorder、AudioWorklet 和 IndexedDB，这些在 jsdom 里都没有，
 * 所以单元测试之外还要有这一层。
 */
const MOCK_PORT = Number(process.env.E2E_MOCK_PORT ?? 18080);
const WEB_PORT = Number(process.env.E2E_WEB_PORT ?? 15174);

// 配置文件会被主进程和每个 worker 各加载一次，临时目录只建一次，靠环境变量传下去
process.env.E2E_TMP_DIR ??= mkdtempSync(join(tmpdir(), "ai-interview-e2e-"));
const tmp = process.env.E2E_TMP_DIR;
const micFile = writeFakeMic(join(tmp, "mic.wav"));
process.env.E2E_MOCK_URL = `http://127.0.0.1:${MOCK_PORT}`;

const media = ["--use-fake-device-for-media-stream", `--use-file-for-fake-audio-capture=${micFile}`, "--autoplay-policy=no-user-gesture-required"];

export default defineConfig({
  testDir: "e2e",
  outputDir: "e2e-results",
  // 模拟服务只有一份状态，测试要一个一个跑
  workers: 1,
  fullyParallel: false,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  reporter: "list",
  use: {
    baseURL: `http://127.0.0.1:${WEB_PORT}`,
    viewport: { width: 390, height: 844 },
    screenshot: "only-on-failure",
    trace: "off",
  },
  projects: [
    {
      name: "允许权限",
      testIgnore: /denied\.spec\.ts/,
      use: { launchOptions: { args: ["--use-fake-ui-for-media-stream", ...media] } },
    },
    {
      name: "拒绝权限",
      testMatch: /denied\.spec\.ts/,
      // 精简版的无头浏览器（headless shell）没有权限弹窗，getUserMedia 会直接报「不支持」。
      // 完整的 Chromium 在没人点「允许」时报的是 NotAllowedError，和候选人点了「拒绝」一样。
      use: { channel: "chromium", launchOptions: { args: media } },
    },
  ],
  webServer: [
    {
      command: `node dev/mock-room-server.mjs --port ${MOCK_PORT} --out ${join(tmp, "recordings")}`,
      url: `http://127.0.0.1:${MOCK_PORT}/__mock/state`,
      reuseExistingServer: false,
    },
    {
      command: `pnpm exec vite --host 127.0.0.1 --port ${WEB_PORT} --strictPort`,
      url: `http://127.0.0.1:${WEB_PORT}/i/`,
      env: { AI_INTERVIEW_API_URL: `http://127.0.0.1:${MOCK_PORT}` },
      reuseExistingServer: false,
    },
  ],
});
