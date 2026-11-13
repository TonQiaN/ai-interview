import { execFileSync } from "node:child_process";
import { expect, test } from "@playwright/test";
import {
  answerTo,
  button,
  enterInterview,
  eventKinds,
  expectQuestion,
  expectRecordingsReady,
  open,
  passPreflight,
  roomState,
  seedInterview,
  spokenKeys,
  UNKNOWN_TOKEN,
} from "./helpers";

/** 没接大模型和语音识别时，一场面试里面试官依次说的话。 */
const SPOKEN_IN_ORDER = [
  "opening",
  "q1",
  "ack.1",
  "q2",
  "ack.2",
  "q3",
  "ack.1",
  "candidate_questions",
  "ack.2",
  "closing",
];

function ffprobeAvailable(): boolean {
  try {
    execFileSync("ffprobe", ["-version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

test("从打开链接到面试完成：逐题问答，静音自动进入下一题，录像完整上传", async ({ page }) => {
  const interview = seedInterview();
  await open(page, interview.token);
  await expect(page.getByText("后端工程师（Rust）")).toBeVisible();
  await button(page, "开始准备").click();

  // 同意事项没有全部勾选之前不能继续
  const agree = button(page, "同意并继续");
  await expect(agree).toBeDisabled();
  const boxes = await page.getByRole("checkbox").all();
  expect(boxes.length).toBeGreaterThanOrEqual(3);
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

  const state = await expectRecordingsReady(interview, 1);
  expect(state.status).toBe("completed");
  expect(state.end_reason).toBe("completed");
  const reports = state.events.filter((event) => event.kind === "device_report");
  expect(reports).toHaveLength(1);
  expect(reports[0]?.payload).toMatchObject({ camera: "ok", microphone: "ok", speaker: "ok" });

  // 面试官的每一句都记了播放起止；每道题答完有一句过渡，最后问候选人有没有问题，再说结束语
  expect(spokenKeys(state)).toEqual(SPOKEN_IN_ORDER);
  const interviewer = state.turns.filter((turn) => turn.speaker === "interviewer");
  for (const turn of interviewer) {
    expect(turn.end_ms).toBeGreaterThan(turn.start_ms);
  }
  // 三道题都是静音自动结束，答题时间在这道题播完之后
  for (const key of ["q1", "q2", "q3"]) {
    const asked = interviewer.find((turn) => turn.utterance_key === key);
    const answer = answerTo(state, key);
    expect(answer?.end_reason, `${key} 的回答`).toBe("silence");
    expect(answer?.start_ms).toBeGreaterThanOrEqual(asked?.end_ms ?? Number.NaN);
    expect(answer?.end_ms).toBeGreaterThan(answer?.start_ms ?? Number.NaN);
  }

  const kinds = eventKinds(state);
  expect(kinds).toContain("interview_started");
  expect(kinds).toContain("interview_finished");

  // 一段录像；有画面有声音，时长盖住整场面试
  const [recording] = state.recordings;
  const lastTurn = interviewer.at(-1);
  if (ffprobeAvailable() && recording?.file) {
    const streams = execFileSync("ffprobe", [
      "-v",
      "error",
      "-show_entries",
      "stream=codec_type",
      "-of",
      "csv=p=0",
      recording.file,
    ]).toString();
    expect(streams).toContain("video");
    expect(streams).toContain("audio");
    expect(recording.duration_ms).toBeGreaterThan((lastTurn?.end_ms ?? 0) - 1500);
  }
});

test("链接过期、被撤销、无效或不完整时，说清楚原因", async ({ page }) => {
  const interview = seedInterview();
  await open(page, interview.expired_token);
  await expect(page.getByRole("heading", { name: "链接已过期" })).toBeVisible();

  await open(page, interview.revoked_token);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接已失效" })).toBeVisible();

  await open(page, UNKNOWN_TOKEN);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接无效" })).toBeVisible();

  await open(page, null);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接不完整" })).toBeVisible();

  expect(roomState(interview).status).toBe("invited");
});

test("可以手动点「我答完了」，也可以提前结束面试", async ({ page }) => {
  const interview = await enterInterview(page);
  await expectQuestion(page, 1);
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);

  await button(page, "结束").click();
  await expect(page.getByText("现在结束面试？")).toBeVisible();
  await button(page, "继续面试").click();
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const state = await expectRecordingsReady(interview, 1);
  expect(state.status).toBe("abandoned");
  expect(state.end_reason).toBe("candidate_left");
  expect(answerTo(state, "q1")?.end_reason).toBe("button");

  // 结束之后再打开链接，不会重新开始，说的还是「已结束」而不是「完成」
  await page.reload();
  await expect(page.getByText("面试已结束")).toBeVisible();
  await expect(button(page, "开始准备")).toHaveCount(0);
});

test("答题中刷新页面，重新检查设备后从中断的那一题继续，两段录像都完整", async ({ page }) => {
  const interview = await enterInterview(page);
  await expectQuestion(page, 2);
  await page.reload();

  await expect(button(page, "检查设备，继续面试")).toBeVisible();
  await passPreflight(page, { resuming: true });
  await button(page, "继续面试").click();
  await expectQuestion(page, 2);
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 60_000 });

  const state = await expectRecordingsReady(interview, 2);
  expect(state.end_reason).toBe("completed");
  // 每句话只记一次：刷新时正在问的第 2 题没有因为重新播了一遍多出一条
  expect(spokenKeys(state)).toEqual(SPOKEN_IN_ORDER);
  // 刷新前一段、刷新后一段；第二段从第一段结束之后开始，时间轴上不重叠
  const [before, after] = state.recordings;
  expect(after?.start_ms).toBeGreaterThan(before?.start_ms ?? Number.NaN);
  if (before?.duration_ms != null) {
    expect(after?.start_ms).toBeGreaterThanOrEqual(before.start_ms + before.duration_ms - 1000);
  }
});

test("面试中断网：继续录制，恢复后补传，一片不少", async ({ page, context }) => {
  // 断网期间不让题目往下走，方便看提示
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const received = () => roomState(interview).recordings[0]?.chunks.length ?? 0;

  await context.setOffline(true);
  await expect(page.getByText("网络不稳定")).toBeVisible({ timeout: 20_000 });
  const before = received();
  await page.waitForTimeout(4000);
  expect(received()).toBeLessThanOrEqual(before + 1);

  await context.setOffline(false);
  await expect(page.getByText("网络不稳定")).toBeHidden({ timeout: 30_000 });
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const state = await expectRecordingsReady(interview, 1);
  // 断网那几秒的分片也在：总片数盖得住整场时长（每片 1 秒）
  expect(state.recordings[0]?.chunks.length ?? 0).toBeGreaterThan(before + 3);
  expect(eventKinds(state)).toEqual(expect.arrayContaining(["network_offline", "network_online"]));
});

test("提交答案时服务暂时不可用，会自动重试，不丢题", async ({ page }) => {
  const interview = await enterInterview(page);
  await expectQuestion(page, 1);
  // 接下来两次上报都让它失败，像后端正在重启
  let failures = 0;
  await page.route("**/api/v1/room/advance", async (route) => {
    if (failures < 2) {
      failures += 1;
      await route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ error: { code: "unavailable", message: "服务暂时不可用" } }),
      });
    } else {
      await route.continue();
    }
  });
  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);
  expect(failures).toBe(2);
  const state = roomState(interview);
  expect(
    state.turns.filter((turn) => turn.utterance_key === "q1" && turn.speaker === "candidate"),
  ).toHaveLength(1);
  expect(spokenKeys(state).filter((key) => key === "q1")).toHaveLength(1);
});

test("切到别的页面会记进过程事件，只记发生过，不记内容", async ({ page }) => {
  const interview = await enterInterview(page);
  await expectQuestion(page, 1);
  await page.evaluate(() => {
    window.dispatchEvent(new Event("blur"));
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new ClipboardEvent("paste"));
  });
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  // 过程事件是攒一小批再报的，结束时会把剩下的发出去
  await expect
    .poll(() => eventKinds(roomState(interview)))
    .toEqual(expect.arrayContaining(["window_blur", "window_focus", "paste"]));
  const paste = roomState(interview).events.find((event) => event.kind === "paste");
  expect(JSON.stringify(paste?.payload ?? {})).not.toMatch(/text|content|data/);
});
