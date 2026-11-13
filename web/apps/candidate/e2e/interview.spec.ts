import { execFileSync } from "node:child_process";
import { expect, test } from "@playwright/test";
import {
  button,
  configureMock,
  enterInterview,
  expectQuestion,
  expectSegmentsComplete,
  mockState,
  open,
  passPreflight,
  resetMock,
  TOKENS,
} from "./helpers";

function ffprobeAvailable(): boolean {
  try {
    execFileSync("ffprobe", ["-version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

test.beforeEach(async () => {
  await resetMock();
});

test("从打开链接到面试完成：逐题问答，静音自动进入下一题，录像完整上传", async ({ page }) => {
  await open(page);
  await expect(page.getByText("后端工程师（Rust）")).toBeVisible();
  await button(page, "开始准备").click();

  // 同意事项没有全部勾选之前不能继续
  const agree = button(page, "同意并继续");
  await expect(agree).toBeDisabled();
  const boxes = await page.getByRole("checkbox").all();
  expect(boxes).toHaveLength(3);
  await boxes[0]?.check();
  await expect(agree).toBeDisabled();
  for (const box of boxes) await box.check();
  await agree.click();

  // 设备检测：三项都通过才能下一步
  await button(page, "允许使用摄像头和麦克风").click();
  await expect(page.getByText("听到你的声音了")).toBeVisible();
  await expect(button(page, "下一步")).toBeDisabled();
  await button(page, "播放提示音").click();
  await button(page, "听到了").click();
  await button(page, "下一步").click();

  await button(page, "开始面试").click();
  await expect(page.getByText("录制中")).toBeVisible();
  await expectQuestion(page, 1);
  await expectQuestion(page, 2);
  await expectQuestion(page, 3);
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 60_000 });

  const state = await mockState();
  expect(state.stage).toBe("finished");
  expect(state.finish_reason).toBe("completed");
  expect(state.device_checks).toHaveLength(1);
  expect(state.device_checks[0]).toMatchObject({ camera: "ok", microphone: "ok", speaker: "ok" });

  // 每一句都报了播放起止；三道题都是静音自动结束，答题时间在播放结束之后
  expect(state.turns.map((turn) => turn.utterance_key)).toEqual(["opening", "q1", "q2", "q3", "closing"]);
  for (const turn of state.turns) {
    expect(turn.playback_end_ms).toBeGreaterThan(turn.playback_start_ms ?? Number.NaN);
  }
  for (const turn of state.turns.filter((item) => item.utterance_key.startsWith("q"))) {
    expect(turn.end_reason).toBe("silence");
    expect(turn.answer_start_ms).toBeGreaterThanOrEqual(turn.playback_end_ms ?? Number.NaN);
    expect(turn.answer_end_ms).toBeGreaterThan(turn.answer_start_ms ?? Number.NaN);
  }

  const kinds = state.events.map((event) => event.kind);
  expect(kinds).toContain("interview_started");
  expect(kinds).toContain("interview_finished");

  // 一段录像，分片齐全；时长盖住整场面试
  expect(state.segments).toHaveLength(1);
  expectSegmentsComplete(state.segments);
  const [segment] = state.segments;
  const lastTurn = state.turns.at(-1);
  if (ffprobeAvailable() && segment?.file) {
    const streams = execFileSync("ffprobe", ["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0", segment.file]).toString();
    expect(streams).toContain("video");
    expect(streams).toContain("audio");
    // 浏览器录出来的文件头里没有时长，数一遍包才知道
    const packets = execFileSync("ffprobe", ["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time", "-of", "csv=p=0", segment.file]).toString().trim().split("\n");
    const durationMs = Number(packets.at(-1)) * 1000;
    expect(durationMs).toBeGreaterThan((lastTurn?.playback_end_ms ?? 0) - 1500);
  }
});

test("链接过期、被撤销、无效或不完整时，说清楚原因", async ({ page }) => {
  await open(page, TOKENS.expired);
  await expect(page.getByRole("heading", { name: "链接已过期" })).toBeVisible();

  await open(page, TOKENS.revoked);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接已失效" })).toBeVisible();

  await open(page, TOKENS.unknown);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接无效" })).toBeVisible();

  await open(page, null);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接不完整" })).toBeVisible();

  expect((await mockState()).stage).toBe("not_started");
});

test("可以手动点「我答完了」，也可以提前结束面试", async ({ page }) => {
  await enterInterview(page);
  await expectQuestion(page, 1);
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);

  await button(page, "结束").click();
  await expect(page.getByText("现在结束面试？")).toBeVisible();
  await button(page, "继续面试").click();
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const state = await mockState();
  expect(state.finish_reason).toBe("candidate_left");
  expect(state.turns.find((turn) => turn.utterance_key === "q1")?.end_reason).toBe("button");
  expectSegmentsComplete(state.segments);

  // 结束之后再打开链接，不会重新开始
  await page.reload();
  await expect(page.getByText("面试已结束").or(page.getByText("面试完成，谢谢你"))).toBeVisible();
});

test("答题中刷新页面，重新检查设备后从中断的那一题继续，两段录像都完整", async ({ page }) => {
  await enterInterview(page);
  await expectQuestion(page, 2);
  await page.reload();

  await expect(button(page, "检查设备，继续面试")).toBeVisible();
  await passPreflight(page, { resuming: true });
  await button(page, "继续面试").click();
  await expectQuestion(page, 2);
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 60_000 });

  const state = await mockState();
  expect(state.finish_reason).toBe("completed");
  expect(state.turns.map((turn) => turn.utterance_key)).toEqual(["opening", "q1", "q2", "q3", "closing"]);
  // 刷新前一段、刷新后一段；第二段的起点在第一段之后
  expect(state.segments).toHaveLength(2);
  expectSegmentsComplete(state.segments);
  const starts = state.segments.map((segment) => segment.start_ms).sort((a, b) => a - b);
  expect(starts[1]).toBeGreaterThan(starts[0] ?? Number.NaN);
});

test("面试中断网：继续录制，恢复后补传，一片不少", async ({ page, context }) => {
  // 断网期间不让题目往下走，方便看提示
  await configureMock({ silenceSec: 30 });
  await enterInterview(page);
  await expectQuestion(page, 1);

  await context.setOffline(true);
  await expect(page.getByText("网络不稳定")).toBeVisible({ timeout: 20_000 });
  const before = (await mockState()).segments[0]?.received.length ?? 0;
  await page.waitForTimeout(4000);
  expect((await mockState()).segments[0]?.received.length ?? 0).toBeLessThanOrEqual(before + 1);

  await context.setOffline(false);
  await expect(page.getByText("网络不稳定")).toBeHidden({ timeout: 30_000 });
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const state = await mockState();
  expect(state.segments).toHaveLength(1);
  expectSegmentsComplete(state.segments);
  // 断网那几秒的分片也在：总片数盖得住整场时长（每片 1 秒）
  expect(state.segments[0]?.received.length ?? 0).toBeGreaterThan(before + 3);
  expect(state.events.map((event) => event.kind)).toEqual(expect.arrayContaining(["network_offline", "network_online"]));
});

test("提交答案时服务暂时不可用，会自动重试，不丢题", async ({ page }) => {
  await enterInterview(page);
  await expectQuestion(page, 1);
  await configureMock({ failAdvance: 2 });
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);
  const state = await mockState();
  expect(state.turns.filter((turn) => turn.utterance_key === "q1")).toHaveLength(1);
});

test("切到别的页面会记进过程事件，只记发生过，不记内容", async ({ page }) => {
  await enterInterview(page);
  await expectQuestion(page, 1);
  await page.evaluate(() => {
    window.dispatchEvent(new Event("blur"));
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new ClipboardEvent("paste"));
  });
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const events = (await mockState()).events;
  const kinds = events.map((event) => event.kind);
  expect(kinds).toEqual(expect.arrayContaining(["window_blur", "window_focus", "paste"]));
  const paste = events.find((event) => event.kind === "paste");
  expect(JSON.stringify(paste?.payload ?? {})).not.toMatch(/text|content|data/);
});
