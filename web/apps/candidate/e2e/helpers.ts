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
  /** 这一段出了什么问题（比如候选人没传完，只有前面一部分）；没有问题是 null */
  error: string | null;
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
export function seedInterview({
  silenceSec = 2,
  minSec = 1,
  maxSec = 60,
  /** 配成实时对话。后端接了真的大模型和语音识别（E2E_REAL=1）时才真的按实时对话进行 */
  realtime = false,
  /** 配成英文面试：题目、岗位名和候选人名字也是英文的 */
  english = false,
} = {}): SeededInterview {
  return demo(
    "seed",
    "--silence-sec",
    String(silenceSec),
    "--min-sec",
    String(minSec),
    "--max-sec",
    String(maxSec),
    "--realtime",
    realtime ? "1" : "0",
    "--english",
    english ? "1" : "0",
  );
}

export const roomState = (interview: SeededInterview): RoomState =>
  demo("state", interview.interview_id);

/** 把这场面试的链接都撤销掉，像 HR 在后台撤销了链接。 */
export const revokeInvites = (interview: SeededInterview): void => {
  demo<{ revoked: number }>("revoke", interview.interview_id);
};

/**
 * 当作候选人在 `minutes` 分钟之前关了页面、再也没回来：还在进行的面试按中途离开结束，
 * 录像最后一次有动静也是那时候。测试不用真的等那么久。
 */
export const walkAway = (interview: SeededInterview, minutes: number): void => {
  demo<{ checks: number }>("walk-away", interview.interview_id, String(minutes));
};

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

/** 页面存录像分片的本地库。 */
export const LOCAL_DB = "ai-interview-recording";
/** 浏览器里现在留着哪些面试的录像：本地键到分片数。页面还没建库时是 null。 */
export const localRecordings = (page: Page): Promise<Record<string, number> | null> =>
  page.evaluate(async (name) => {
    const existing = await indexedDB.databases();
    if (!existing.some((item) => item.name === name)) return null;
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    try {
      const transaction = db.transaction(["segments", "chunk-index"], "readonly");
      const all = <T>(store: string) =>
        new Promise<T[]>((resolve, reject) => {
          const request = transaction.objectStore(store).getAll();
          request.onsuccess = () => resolve(request.result as T[]);
          request.onerror = () => reject(request.error);
        });
      const segments = await all<{ sessionKey: string }>("segments");
      const chunks = await all<{ sessionKey: string }>("chunk-index");
      const found: Record<string, number> = {};
      for (const segment of segments) found[segment.sessionKey] = 0;
      for (const chunk of chunks) found[chunk.sessionKey] = (found[chunk.sessionKey] ?? 0) + 1;
      return found;
    } finally {
      db.close();
    }
  }, LOCAL_DB);

/** 页面给一条链接用的本地键：令牌摘要的前 32 位。 */
export const sessionKeyOf = (page: Page, token: string): Promise<string> =>
  page.evaluate(async (value) => {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
    return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 32);
  }, token);
