import { expect, type Page, test } from "@playwright/test";
import { button, open, seedInterview } from "./helpers";

/**
 * 面试页在手机浏览器里的版面。
 *
 * 手机上浏览器的地址栏和工具栏要占掉一截，真正看得到的高度比屏幕矮不少
 * （iPhone 13 到 15 的 Safari 是 390×664，iPhone SE 是 375×548）。
 * 候选人答题时手不在屏幕上，按钮、倒计时、提示都得在第一屏里，不能要往下滑才看得到。
 */

const ZH = {
  ready: "开始准备",
  agree: "同意并继续",
  allow: "允许使用摄像头和麦克风",
  play: "播放提示音",
  heard: "听到了",
  next: "下一步",
  start: "开始面试",
  recording: "录制中",
  done: "我答完了，下一题",
  notDone: "我还没说完",
  countdownHint: "说完了吗？还想补充就继续说",
  countdown: /秒后进入下一题/,
};
const EN: typeof ZH = {
  ready: "Get ready",
  agree: "Agree and continue",
  allow: "Allow camera and microphone",
  play: "Play test sound",
  heard: "I heard it",
  next: "Next",
  start: "Start interview",
  recording: "Recording",
  done: "I'm done, next question",
  notDone: "I'm not done yet",
  countdownHint: "Finished? Keep talking if you have more to add",
  countdown: /Next question in/,
};

/** 走到面试开始。静音 3 秒算答完，倒计时也是 3 秒：假麦克风一停下来倒计时就出来。 */
async function enter(page: Page, english: boolean): Promise<typeof ZH> {
  const t = english ? EN : ZH;
  const interview = seedInterview({ english, silenceSec: 3 });
  await open(page, interview.token);
  await button(page, t.ready).click();
  for (const box of await page.getByRole("checkbox").all()) await box.check();
  await button(page, t.agree).click();
  await button(page, t.allow).click();
  await button(page, t.play).click();
  await button(page, t.heard).click();
  await button(page, t.next).click();
  await button(page, t.start).click();
  await expect(page.getByText(t.recording, { exact: true })).toBeVisible();
  return t;
}

/** 有哪些看得见的东西伸到了屏幕左右两边之外（字被切掉、页面能左右滑）。 */
const offScreen = (page: Page): Promise<string[]> =>
  page.evaluate(() => {
    const width = document.documentElement.clientWidth;
    const found: string[] = [];
    if (document.documentElement.scrollWidth > width + 1) found.push("页面能左右滑动");
    for (const element of document.body.querySelectorAll<HTMLElement>("*")) {
      const rect = element.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      // 作答按钮的底色是有意铺到屏幕边上的，里面没有字
      if (element.classList.contains("app-foot")) continue;
      if (rect.right > width + 1 || rect.left < -1) {
        const text = (element.textContent ?? "").trim().slice(0, 40);
        found.push(`<${element.tagName.toLowerCase()}> ${text}`);
      }
    }
    return found;
  });

/** 页面比屏幕高出多少像素（要往下滑多少才看得全）。 */
const overflowY = (page: Page): Promise<number> =>
  page.evaluate(() => document.documentElement.scrollHeight - window.innerHeight);

for (const { name, viewport, english, fits } of [
  {
    name: "iPhone 的 Safari（390×664）",
    viewport: { width: 390, height: 664 },
    english: false,
    fits: true,
  },
  {
    name: "iPhone 的 Safari，英文面试",
    viewport: { width: 390, height: 664 },
    english: true,
    fits: true,
  },
  {
    name: "安卓的 Chrome（412×760）",
    viewport: { width: 412, height: 760 },
    english: false,
    fits: true,
  },
  // 再小的屏幕一屏放不下，题目可以滚，但按钮和倒计时还是得在第一屏
  {
    name: "iPhone SE 的 Safari（375×548）",
    viewport: { width: 375, height: 548 },
    english: false,
    fits: false,
  },
  {
    name: "很窄的手机，英文面试（320×568）",
    viewport: { width: 320, height: 568 },
    english: true,
    fits: false,
  },
  {
    name: "手机横过来（667×375）",
    viewport: { width: 667, height: 375 },
    english: true,
    fits: false,
  },
]) {
  test.describe(name, () => {
    test.use({ viewport });

    test(`${name}：答题的按钮、倒计时和提示都在第一屏`, async ({ page }) => {
      const t = await enter(page, english);

      // 开始回答：按钮在第一屏，自己的画面也在
      const done = button(page, t.done);
      await expect(done).toBeVisible({ timeout: 30_000 });
      await expect(done).toBeInViewport({ ratio: 1 });
      // 画面被画框裁掉一点边，所以不是整整 1
      await expect(page.locator("video")).toBeInViewport({ ratio: 0.95 });
      expect(await offScreen(page), "回答时").toEqual([]);

      // 停下来以后：倒计时、「还没说完」和那句提示都看得到
      const notDone = button(page, t.notDone);
      await expect(notDone).toBeVisible({ timeout: 30_000 });
      await expect(notDone).toBeInViewport({ ratio: 1 });
      await expect(done).toBeInViewport({ ratio: 1 });
      await expect(page.getByRole("timer", { name: t.countdown })).toBeInViewport({ ratio: 1 });
      await expect(page.getByText(t.countdownHint)).toBeInViewport({ ratio: 1 });
      expect(await offScreen(page), "倒计时的时候").toEqual([]);

      if (fits) expect(await overflowY(page), "整页应该一屏放得下").toBeLessThanOrEqual(0);
    });
  });
}
