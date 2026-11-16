import { expect, type Page, test } from "@playwright/test";
import { button, expectRecordingsReady, open, seedInterview, UNKNOWN_TOKEN } from "./helpers";

/**
 * 只用键盘、靠读屏软件的候选人也能走完一场面试。
 *
 * 这里没有真的读屏软件，查的是读屏软件靠的那几样东西：换页以后焦点在这一页的标题上
 * （读屏软件会把它读出来）、步骤条读成一句话、浏览器标签页上的名字、页面有主要内容区域。
 * 全程不点鼠标。
 */

/** 现在在这一页：焦点在标题上，标签页的名字跟着换了。 */
async function expectPage(page: Page, heading: string, title: string): Promise<void> {
  const h1 = page.getByRole("main").getByRole("heading", { level: 1, name: heading });
  await expect(h1).toBeFocused({ timeout: 30_000 });
  await expect(page).toHaveTitle(title);
}

/** 按 Tab 走到某个按钮上，再按回车。 */
async function pressButton(page: Page, name: string): Promise<void> {
  const target = button(page, name);
  await expect(target).toBeEnabled({ timeout: 30_000 });
  for (let i = 0; i < 12; i++) {
    if (await target.evaluate((el) => el === document.activeElement)) break;
    await page.keyboard.press("Tab");
  }
  await expect(target).toBeFocused();
  await page.keyboard.press("Enter");
}

const step = (page: Page, name: string) => page.getByRole("img", { name, exact: true });

test("只用键盘走完一场面试：每换一页焦点都在标题上，步骤条和标签页的名字说得清在哪一步", async ({
  page,
}) => {
  const interview = seedInterview();
  await open(page, interview.token);

  // 欢迎页的标题里有候选人的名字，标签页上不带名字
  await expectPage(page, "测试候选人，你好", "须知 · AI 面试");
  await expect(step(page, "第 1 步，共 4 步：须知")).toBeVisible();
  await pressButton(page, "开始准备");

  await expectPage(page, "开始之前，请确认", "同意 · AI 面试");
  await expect(step(page, "第 2 步，共 4 步：同意")).toBeVisible();
  const boxes = await page.getByRole("checkbox").all();
  expect(boxes.length).toBeGreaterThan(0);
  for (const box of boxes) {
    for (let i = 0; i < 6; i++) {
      if (await box.evaluate((el) => el === document.activeElement)) break;
      await page.keyboard.press("Tab");
    }
    await expect(box).toBeFocused();
    await page.keyboard.press("Space");
    await expect(box).toBeChecked();
  }
  await pressButton(page, "同意并继续");

  await expectPage(page, "检查设备", "设备 · AI 面试");
  await expect(step(page, "第 3 步，共 4 步：设备")).toBeVisible();
  await pressButton(page, "允许使用摄像头和麦克风");
  await pressButton(page, "播放提示音");
  await pressButton(page, "听到了");
  await pressButton(page, "下一步");

  await expectPage(page, "准备好就开始", "开始 · AI 面试");
  await expect(step(page, "第 4 步，共 4 步：开始")).toBeVisible();
  await pressButton(page, "开始面试");

  // 面试中那一页画面上没有标题，读屏软件有
  await expectPage(page, "面试进行中", "面试进行中 · AI 面试");
  await expect(page.getByText("录制中")).toBeVisible();
  // 面试官说话的时候读屏软件不念提示（会盖住题目的开头），轮到候选人回答时才念
  const spoken = page.locator('p[role="status"]');
  await expect(page.locator(".oil-presence__label")).toHaveText("正在提问");
  await expect(spoken).toHaveText("");
  await expect(spoken).toHaveText("请回答", { timeout: 30_000 });
  await pressButton(page, "我答完了，下一题");

  // 结束面试：确认的弹窗打开时焦点进弹窗，按 Esc 关掉后回到「结束」上
  await pressButton(page, "结束");
  const dialog = page.getByRole("dialog", { name: "现在结束面试？" });
  await expect(dialog).toBeVisible();
  expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(button(page, "结束")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dialog).toBeVisible();
  await pressButton(page, "结束面试");

  await expectPage(page, "面试已结束", "面试已结束 · AI 面试");
  await expectRecordingsReady(interview, 1);
});

test("链接无效的那一页：焦点在标题上，标签页上写着出了什么事", async ({ page }) => {
  await open(page, UNKNOWN_TOKEN);
  await expectPage(page, "链接无效", "链接无效 · AI 面试");
});
