import { execFileSync } from "node:child_process";
import { expect, type Page } from "@playwright/test";

/** 造出来的一场面试：题目和语音都备好了，链接已经发出。 */
export interface SeededInterview {
  interview_id: string;
  /** 能用的链接令牌 */
  token: string;
  /** 同一场面试的一条过期了的链接 */
  expired_token: string;
  /** 同一场面试的一条撤销了的链接 */
  revoked_token: string;
}

export interface RoomTurn {
  seq: number;
  speaker: "interviewer" | "candidate";
  utterance_key: string | null;
  start_ms: number;
  end_ms: number | null;
  end_reason: string | null;
  /** 这一句有没有文字。候选人的话要转写出来才有 */
  has_text: boolean;
  /** 候选人的话是哪个识别后端转写的 */
  stt_engine: string | null;
}

export interface RoomRecording {
  track: string;
  segment: number;
  content_type: string;
  status: string;
  start_ms: number;
  duration_ms: number | null;
  total_chunks: number | null;
  size_bytes: number | null;
  /** 收到了哪几片 */
  chunks: number[];
  /** 合并好的文件在磁盘上的位置 */
  file: string | null;
}

/** 后端库里这场面试现在的样子。 */
export interface RoomState {
  status: string;
  end_reason: string | null;
  turns: RoomTurn[];
  events: {
    kind: string;
    source: string;
    at_ms: number | null;
    payload: Record<string, unknown>;
  }[];
  recordings: RoomRecording[];
}

/** 跑造数据的小工具（crates/interview-room/examples/demo.rs），它和后端用同一个库和数据目录。 */
function demo<T>(...args: string[]): T {
  const bin = process.env.E2E_DEMO_BIN;
  const env = process.env.E2E_BACKEND_ENV;
  if (!bin || !env) throw new Error("没有后端的环境信息，要通过 playwright.config.ts 来跑");
  const output = execFileSync(bin, args, {
    env: { ...process.env, ...JSON.parse(env) },
    encoding: "utf8",
  });
  return JSON.parse(output) as T;
}

/**
 * 造一场面试。答题静音时长默认 2 秒：假麦克风每 5 秒里说 2.5 秒、安静 2.5 秒，
 * 所以每道题大约 5 秒答完，测试跑得快。
 */
export function seedInterview({ silenceSec = 2, minSec = 1, maxSec = 60 } = {}): SeededInterview {
  return demo(
    "seed",
    "--silence-sec",
    String(silenceSec),
    "--min-sec",
    String(minSec),
    "--max-sec",
    String(maxSec),
  );
}

export const roomState = (interview: SeededInterview): RoomState =>
  demo("state", interview.interview_id);

/** 面试官依次说了哪几句。 */
export const spokenKeys = (state: RoomState): (string | null)[] =>
  state.turns.filter((turn) => turn.speaker === "interviewer").map((turn) => turn.utterance_key);

/** 候选人回答某一句的那一段。 */
export const answerTo = (state: RoomState, key: string): RoomTurn | undefined =>
  state.turns.find((turn) => turn.speaker === "candidate" && turn.utterance_key === key);

export const eventKinds = (state: RoomState): string[] => state.events.map((event) => event.kind);

/** 一个格式正确但后端不认识的令牌。 */
export const UNKNOWN_TOKEN = "unknown-token-that-nobody-issued-0000000001";

export const open = (page: Page, token: string | null) =>
  page.goto(token === null ? "/i/" : `/i/#${token}`);

export const button = (page: Page, name: string) => page.getByRole("button", { name, exact: true });

/** 从欢迎页走到「准备好就开始」那一页。 */
export async function passPreflight(page: Page, { resuming = false } = {}): Promise<void> {
  await button(page, resuming ? "检查设备，继续面试" : "开始准备").click();
  if (!resuming) {
    for (const box of await page.getByRole("checkbox").all()) await box.check();
    await button(page, "同意并继续").click();
  }
  await button(page, "允许使用摄像头和麦克风").click();
  await button(page, "播放提示音").click();
  await button(page, "听到了").click();
  await button(page, "下一步").click();
}

/** 造一场面试，从打开链接一路走到面试开始。 */
export async function enterInterview(
  page: Page,
  options: Parameters<typeof seedInterview>[0] = {},
): Promise<SeededInterview> {
  const interview = seedInterview(options);
  await open(page, interview.token);
  await passPreflight(page);
  await button(page, "开始面试").click();
  await expect(page.getByText("录制中")).toBeVisible();
  return interview;
}

/** 等到正在问某一题。 */
export const expectQuestion = (page: Page, index: number, total = 3) =>
  expect(page.getByText(`第 ${index} 题 / 共 ${total} 题`)).toBeVisible({ timeout: 30_000 });

/**
 * 等后端把每一段录像都合并好，返回那时的状态。合并是后台任务，面试结束后要等一小会儿。
 * 每一段都收齐了分片：片号从 0 连到 total_chunks - 1，一片不少。
 */
export async function expectRecordingsReady(
  interview: SeededInterview,
  count: number,
): Promise<RoomState> {
  await expect
    .poll(() => roomState(interview).recordings.map((recording) => recording.status), {
      timeout: 30_000,
    })
    .toEqual(Array.from({ length: count }, () => "ready"));
  const state = roomState(interview);
  for (const recording of state.recordings) {
    expect(recording.total_chunks, `第 ${recording.segment} 段没有说一共多少片`).not.toBeNull();
    expect(recording.chunks).toEqual(
      Array.from({ length: recording.total_chunks ?? 0 }, (_, seq) => seq),
    );
    expect(recording.size_bytes).toBeGreaterThan(0);
  }
  return state;
}
