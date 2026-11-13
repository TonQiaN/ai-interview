import { expect, type Page } from "@playwright/test";

/** 模拟服务认识的几个假令牌，和 dev/mock-room-server.mjs 里的一致。 */
export const TOKENS = {
  valid: "mock-interview-token-000001",
  expired: "mock-expired-token-00000001",
  revoked: "mock-revoked-token-00000001",
  unknown: "mock-unknown-token-00000001",
};

const mockUrl = () => process.env.E2E_MOCK_URL ?? "http://127.0.0.1:18080";

export interface MockSegment {
  segment_id: string;
  track: string;
  content_type: string;
  start_ms: number;
  received: number[];
  bytes: number;
  total_chunks: number | null;
  file: string | null;
}

export interface MockTurn {
  utterance_key: string;
  playback_start_ms: number | null;
  playback_end_ms: number | null;
  answer_start_ms: number | null;
  answer_end_ms: number | null;
  end_reason: string | null;
  replays: number;
}

export interface MockState {
  stage: string;
  consent_agreed: boolean;
  index: number;
  turns: MockTurn[];
  events: { kind: string; at_ms: number | null; payload: Record<string, unknown> }[];
  device_checks: Record<string, unknown>[];
  segments: MockSegment[];
  finish_reason: string | null;
}

/** 清空模拟服务的状态，并按需要调整它的行为。答题静音时长默认调短，测试跑得快一些。 */
export async function resetMock(config: Record<string, unknown> = {}): Promise<void> {
  await fetch(`${mockUrl()}/__mock/reset`, { method: "POST" });
  await configureMock({ silenceSec: 1.5, ...config });
}

export async function configureMock(config: Record<string, unknown>): Promise<void> {
  await fetch(`${mockUrl()}/__mock/config`, { method: "POST", body: JSON.stringify(config) });
}

export async function mockState(): Promise<MockState> {
  return (await (await fetch(`${mockUrl()}/__mock/state`)).json()) as MockState;
}

export const open = (page: Page, token: string | null = TOKENS.valid) => page.goto(token === null ? "/i/" : `/i/#${token}`);

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

/** 从打开链接一路走到面试开始。 */
export async function enterInterview(page: Page): Promise<void> {
  await open(page);
  await passPreflight(page);
  await button(page, "开始面试").click();
  await expect(page.getByText("录制中")).toBeVisible();
}

/** 等到正在问某一题。 */
export const expectQuestion = (page: Page, index: number, total = 3) =>
  expect(page.getByText(`第 ${index} 题 / 共 ${total} 题`)).toBeVisible({ timeout: 30_000 });

/** 每一段录像都收齐了分片并且完成了合并。 */
export function expectSegmentsComplete(segments: MockSegment[]): void {
  for (const segment of segments) {
    expect(segment.total_chunks, `段 ${segment.segment_id} 没有合并`).not.toBeNull();
    expect(segment.received).toEqual(Array.from({ length: segment.total_chunks ?? 0 }, (_, seq) => seq));
    expect(segment.bytes).toBeGreaterThan(0);
  }
}
