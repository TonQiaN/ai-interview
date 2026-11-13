import { execFileSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { defineConfig } from "@playwright/test";
import { writeFakeMic, writeSpeechMic } from "./e2e/fake-mic";

/**
 * 浏览器端到端测试：真的 Chromium，假的摄像头和麦克风，真的后端。
 *
 *   pnpm e2e                 # 全部
 *   pnpm e2e -g 刷新         # 只跑名字里带「刷新」的
 *
 * 用到摄像头、麦克风、MediaRecorder、AudioWorklet 和 IndexedDB，这些在 jsdom 里都没有，
 * 所以单元测试之外还要有这一层。
 *
 * 后端由 e2e/backend.sh 起：用开发数据库实例（scripts/dev-db.sh）上单独的一个库，每次清空重建。
 * 每个测试自己造一场面试（crates/interview-room/examples/demo.rs），互不影响。
 * 需要本机有 Rust 工具链和 PostgreSQL；第一次跑要先编译后端，会慢一些。
 *
 * 平时语音识别和大模型都是假的。想用真的走一遍（会花一点 DeepSeek 的钱，CI 不跑）：
 *
 *   E2E_REAL=1 E2E_SPEECH_WAV=<一段有人说话的 WAV> pnpm e2e
 *
 * 这时只跑 e2e/real.spec.ts，假麦克风放的是那段 WAV。还要有 DEEPSEEK_API_KEY 和
 * AI_INTERVIEW_STT_MODEL_DIR（见 crates/interview-stt/README.md），后端会带着 sherpa 特性编译。
 * 识别模型的包里带了几段示例录音（test_wavs/），可以直接拿来用。
 */
const REAL = process.env.E2E_REAL === "1";
const API_PORT = Number(process.env.E2E_API_PORT ?? 18080);
const WEB_PORT = Number(process.env.E2E_WEB_PORT ?? 15174);
const repoRoot = resolve(import.meta.dirname, "../../..");

// 配置文件会被主进程和每个 worker 各加载一次，临时目录只建一次，靠环境变量传下去
process.env.E2E_TMP_DIR ??= mkdtempSync(join(tmpdir(), "ai-interview-e2e-"));
const tmp = process.env.E2E_TMP_DIR;
const micFile = REAL ? speechMic(join(tmp, "mic.wav")) : writeFakeMic(join(tmp, "mic.wav"));

function speechMic(path: string): string {
  const speech = process.env.E2E_SPEECH_WAV;
  if (!speech) throw new Error("E2E_REAL=1 时要用 E2E_SPEECH_WAV 指一段有人说话的 WAV");
  return writeSpeechMic(path, speech);
}

process.env.E2E_DATABASE_URL ??= execFileSync(join(repoRoot, "scripts/dev-db.sh"), ["url"], {
  encoding: "utf8",
})
  .trim()
  .replace(/\/[^/]+$/, "/ai_interview_e2e");
/** 后端和造数据的小工具共用的环境变量：同一个库、同一个数据目录。 */
const backendEnv = {
  DATABASE_URL: process.env.E2E_DATABASE_URL,
  AI_INTERVIEW_DATA_DIR: join(tmp, "data"),
  AI_INTERVIEW_BIND: `127.0.0.1:${API_PORT}`,
  AI_INTERVIEW_PUBLIC_URL: `http://127.0.0.1:${WEB_PORT}`,
};
process.env.E2E_BACKEND_ENV = JSON.stringify(backendEnv);
process.env.E2E_DEMO_BIN = join(repoRoot, "target/debug/examples/demo");

const media = [
  "--use-fake-device-for-media-stream",
  `--use-file-for-fake-audio-capture=${micFile}`,
  "--autoplay-policy=no-user-gesture-required",
];

export default defineConfig({
  testDir: "e2e",
  outputDir: "e2e-results",
  // 录像和静音检测都吃 CPU，一个一个跑时间才稳
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
  projects: REAL
    ? [
        {
          name: "真模型",
          testMatch: /real\.spec\.ts/,
          timeout: 10 * 60_000,
          use: { launchOptions: { args: ["--use-fake-ui-for-media-stream", ...media] } },
        },
      ]
    : [
        {
          name: "允许权限",
          testIgnore: [/denied\.spec\.ts/, /real\.spec\.ts/],
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
      command: "e2e/backend.sh",
      // 没带令牌，返回 401 就说明服务起来了
      url: `http://127.0.0.1:${API_PORT}/api/v1/room/session`,
      env: backendEnv,
      // 用真模型跑的时候把后端的日志也打出来（AI_INTERVIEW_LOG=info 能看到每一轮各段用了多久）
      stdout: REAL ? "pipe" : "ignore",
      // 第一次要编译后端
      timeout: 15 * 60_000,
      reuseExistingServer: false,
    },
    {
      command: `pnpm exec vite --host 127.0.0.1 --port ${WEB_PORT} --strictPort`,
      url: `http://127.0.0.1:${WEB_PORT}/i/`,
      env: { AI_INTERVIEW_API_URL: `http://127.0.0.1:${API_PORT}` },
      reuseExistingServer: false,
    },
  ],
});
