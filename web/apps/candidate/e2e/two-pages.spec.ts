import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  eventKinds,
  expectQuestion,
  expectRecordingsReady,
  open,
  passPreflight,
  roomState,
  type SeededInterview,
  seedInterview,
} from "./helpers";

/**
 * 同一条面试链接同时开在两个页面里（这里是两个互不相干的浏览器环境，像手机和电脑各开一个）。
 * 后点「继续面试」的那个页面接着面，前一个停下来，把它录下的传完。
 */

const MOVED = "面试已经在另一个页面里继续了";

const movedNotice = (page: Page) => page.getByRole("heading", { name: MOVED });

/** 在另一个页面打开同一条链接，走到「继续面试」那一步，先不点。 */
async function openElsewhere(other: Page, interview: SeededInterview): Promise<void> {
  await open(other, interview.token);
  await passPreflight(other, { resuming: true });
}

const takenOver = (interview: SeededInterview): number =>
  eventKinds(roomState(interview)).filter((kind) => kind === "page_taken_over").length;

test("在另一个页面继续面试：面试换到那边，原来的页面停下来，录像一段不少", async ({
  page,
  browser,
}) => {
  // 静音 30 秒才算答完：题目不会自己往下走
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);

  const context = await browser.newContext();
  const other = await context.newPage();
  await openElsewhere(other, interview);
  // 那边只是打开了链接、检查了设备：这边照常面试
  await page.waitForTimeout(3000);
  await expect(page.getByText("录制中")).toBeVisible();
  await expect(movedNotice(page)).toHaveCount(0);

  await button(other, "继续面试").click();
  await expect(other.getByText("录制中")).toBeVisible();
  await expectQuestion(other, 1);

  // 这边几秒之内就停下来，把录下的传完
  await expect(movedNotice(page)).toBeVisible();
  await expect(page.getByText("录制中")).toBeHidden();
  await expect(button(page, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });
  expect(roomState(interview).status).toBe("in_progress");
  expect(takenOver(interview)).toBe(1);

  // 面试在那边接着往下走
  await button(other, "我答完了，下一题").click();
  await expectQuestion(other, 2);
  await button(other, "结束").click();
  await button(other, "结束面试").click();
  await expect(other.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  // 两个页面各一段录像，都完整；第一题只问了一次、答了一次
  const state = await expectRecordingsReady(interview, 2);
  expect(state.end_reason).toBe("candidate_left");
  const first = state.turns.filter((turn) => turn.utterance_key === "q1");
  expect(first.map((turn) => turn.speaker)).toEqual(["interviewer", "candidate"]);
  const [before, after] = state.recordings;
  expect(after?.start_ms).toBeGreaterThan(before?.start_ms ?? Number.NaN);
  // 原来的页面停了之后没有再录：它那一段比整场面试短
  expect(before?.chunks.length ?? 0).toBeGreaterThan(0);
  expect(takenOver(interview)).toBe(1);
  await context.close();
});

test("换到另一个页面之后可以再回来：点一下，重新检查设备，面试回到这个页面", async ({
  page,
  browser,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);

  const context = await browser.newContext();
  const other = await context.newPage();
  await openElsewhere(other, interview);
  await button(other, "继续面试").click();
  await expect(other.getByText("录制中")).toBeVisible();
  await expect(button(page, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });

  await button(page, "回到这个页面继续面试").click();
  await passPreflight(page, { resuming: true });
  await button(page, "继续面试").click();
  await expect(page.getByText("录制中")).toBeVisible();
  await expectQuestion(page, 1);
  // 这回轮到那边停下来
  await expect(movedNotice(other)).toBeVisible();
  await expect(button(other, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });

  await button(page, "我答完了，下一题").click();
  await expectQuestion(page, 2);
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  await expectRecordingsReady(interview, 3);
  expect(takenOver(interview)).toBe(2);
  await context.close();
});

test("留在后面的页面点「结束面试」结束不了面试：它被告知面试在另一个页面里", async ({
  page,
  browser,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  // 等到真的在听第一题的回答（说开场白的时候题号也是 1）：之后这个页面不会自己再去问服务端下一步
  await expect(button(page, "我答完了，下一题")).toBeVisible();
  // 这个页面没来得及从别的响应里知道面试换了地方：把提醒去掉，只留结束面试那一个请求的
  await page.route("**/api/v1/room/**", async (route) => {
    const response = await route.fetch();
    const headers = response.headers();
    if (!route.request().url().endsWith("/finish")) delete headers["x-room-active-elsewhere"];
    await route.fulfill({ response, headers });
  });

  const context = await browser.newContext();
  const other = await context.newPage();
  await openElsewhere(other, interview);
  await button(other, "继续面试").click();
  await expect(other.getByText("录制中")).toBeVisible();

  await expect(page.getByText("录制中")).toBeVisible();
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(movedNotice(page)).toBeVisible({ timeout: 30_000 });
  await expect(button(page, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });

  // 面试没有被结束掉，那边还在面
  expect(roomState(interview).status).toBe("in_progress");
  await expect(other.getByText("录制中")).toBeVisible();
  await expect(movedNotice(other)).toHaveCount(0);
  await page.unrouteAll({ behavior: "ignoreErrors" });
  await context.close();
});

test("往下走的请求正好撞上面试换了页面，提醒又没送到：照样告诉候选人面试在另一个页面里", async ({
  page,
  browser,
}) => {
  const interview = seedInterview({ silenceSec: 30 });
  // 提醒用的响应头一个都到不了这个页面（像被中间的代理去掉了）；开场白说完往下走的那个请求先按住
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/room/**", async (route) => {
    if (route.request().url().endsWith("/advance")) await held;
    const response = await route.fetch();
    const headers = response.headers();
    delete headers["x-room-active-elsewhere"];
    await route.fulfill({ response, headers });
  });
  await open(page, interview.token);
  await passPreflight(page);
  await button(page, "开始面试").click();
  await expect(page.getByText("录制中")).toBeVisible();

  const context = await browser.newContext();
  const other = await context.newPage();
  await openElsewhere(other, interview);
  await button(other, "继续面试").click();
  await expect(other.getByText("录制中")).toBeVisible();

  // 这时候那个请求才到服务端：服务端不让它往下走。页面不能说成「链接已经失效」
  release();
  await expect(movedNotice(page)).toBeVisible();
  await expect(button(page, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });
  expect(roomState(interview).status).toBe("in_progress");
  expect(takenOver(interview)).toBe(1);
  await expect(other.getByText("录制中")).toBeVisible();
  await page.unrouteAll({ behavior: "ignoreErrors" });
  await context.close();
});

test("同一个浏览器里开两个标签页：各管各的录像，先开的那个页面录下的一片不少", async ({
  page,
  context,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  const startedAt = Date.now();
  await expectQuestion(page, 1);
  const first = () => roomState(interview).recordings[0];

  // 同一个浏览器里的第二个标签页：和第一个共用本机的录像队列
  const tab = await context.newPage();
  await openElsewhere(tab, interview);
  // 它只是打开了链接：第一个页面正在录的那一段不能被它收尾，之后录的照样传得上去
  const before = first()?.chunks.length ?? 0;
  await page.waitForTimeout(4000);
  expect(first()?.total_chunks ?? null).toBeNull();
  expect(first()?.chunks.length ?? 0).toBeGreaterThan(before + 1);
  await expect(page.getByText("录制中")).toBeVisible();

  await button(tab, "继续面试").click();
  await expect(tab.getByText("录制中")).toBeVisible();
  await expect(movedNotice(page)).toBeVisible();
  const stoppedAt = Date.now();
  // 第一个页面传完自己的录像就显示完成，不等第二个页面还在录的那一段
  await expect(button(page, "回到这个页面继续面试")).toBeVisible({ timeout: 30_000 });
  // 它那一段这时候已经完整地在服务端了：每秒一片，盖得住它面试的那段时间
  await expect.poll(() => first()?.status, { timeout: 30_000 }).toBe("ready");
  const seconds = Math.floor((stoppedAt - startedAt) / 1000);
  expect(first()?.total_chunks ?? 0).toBeGreaterThanOrEqual(seconds - 2);
  expect(first()?.chunks.length).toBe(first()?.total_chunks);

  await button(tab, "我答完了，下一题").click();
  await expectQuestion(tab, 2);
  await button(tab, "结束").click();
  await button(tab, "结束面试").click();
  await expect(tab.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });
  await expectRecordingsReady(interview, 2);
  expect(takenOver(interview)).toBe(1);
});

test("同一个浏览器里先开的标签页关掉了：它留下的录像由后开的页面接着传完", async ({
  page,
  context,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const first = () => roomState(interview).recordings[0];

  const tab = await context.newPage();
  await openElsewhere(tab, interview);
  // 断网之后再录几秒：这几片只在本机的队列里
  await context.setOffline(true);
  await page.waitForTimeout(4000);
  const received = first()?.chunks.length ?? 0;
  await page.close();
  await context.setOffline(false);

  // 后开的页面一开始面试就把它收尾、传完，不用等到面试结束
  await button(tab, "继续面试").click();
  await expect(tab.getByText("录制中")).toBeVisible();
  await expect.poll(() => first()?.status, { timeout: 30_000 }).toBe("ready");
  expect(first()?.total_chunks ?? 0).toBeGreaterThan(received + 2);
  expect(first()?.chunks.length).toBe(first()?.total_chunks);
  expect(roomState(interview).status).toBe("in_progress");

  await button(tab, "结束").click();
  await button(tab, "结束面试").click();
  await expect(tab.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });
  await expectRecordingsReady(interview, 2);
  // 先开的页面是关掉的，不是被接走的
  expect(takenOver(interview)).toBe(0);
});
