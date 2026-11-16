import { expect, type Page, test } from "@playwright/test";
import {
  answerTo,
  button,
  expectRecordingsReady,
  open,
  seedInterview,
  UNKNOWN_TOKEN,
} from "./helpers";

/** 页面上（看得见的字、读屏文字、图片说明）一个中文字都没有。 */
async function expectNoChinese(page: Page, where: string): Promise<void> {
  const leftovers = await page.evaluate(() => {
    const html = document.body.innerHTML;
    return [...html.matchAll(/[　-〿一-鿿＀-￯]+/g)].map((match) =>
      html.slice(Math.max(0, (match.index ?? 0) - 40), (match.index ?? 0) + match[0].length + 10),
    );
  });
  expect(leftovers, `${where}还有中文`).toEqual([]);
}

test("英文面试：从须知到结束页都是英文", async ({ page }) => {
  const interview = seedInterview({ english: true });
  await open(page, interview.token);

  await expect(page.getByText("Backend Engineer (Rust)")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Hello, Test Candidate" })).toBeVisible();
  await expect(page).toHaveTitle("AI Interview");
  expect(await page.evaluate(() => document.documentElement.lang)).toBe("en");
  await expectNoChinese(page, "须知页");
  await button(page, "Get ready").click();

  // 同意事项是服务端按面试语言给的
  await expect(page.getByText("I agree to this interview being audio and video recorded")).toBeVisible();
  for (const box of await page.getByRole("checkbox").all()) await box.check();
  await page.getByRole("button", { name: "Show details" }).first().click();
  await expectNoChinese(page, "同意页");
  await button(page, "Agree and continue").click();

  await expect(page.getByText("Upload speed is good enough.")).toBeVisible();
  await expectNoChinese(page, "设备检测页（授权前）");
  await button(page, "Allow camera and microphone").click();
  await expect(page.getByText("We can hear you.")).toBeVisible();
  await button(page, "Play test sound").click();
  await button(page, "I heard it").click();
  await expectNoChinese(page, "设备检测页");
  await button(page, "Next").click();

  await expect(page.getByRole("heading", { name: "Start when you are ready" })).toBeVisible();
  await expectNoChinese(page, "准备页");
  await button(page, "Start interview").click();

  await expect(page.getByText("Recording", { exact: true })).toBeVisible();
  await expect(page.getByText("Question 1 of 3")).toBeVisible({ timeout: 30_000 });
  await expectNoChinese(page, "面试页（提问）");
  const done = button(page, "I'm done, next question");
  await expect(done).toBeVisible({ timeout: 30_000 });
  await expectNoChinese(page, "面试页（回答）");
  await done.click();
  await expect(page.getByText("Question 2 of 3")).toBeVisible({ timeout: 30_000 });

  await button(page, "End").click();
  await expect(page.getByText("End the interview now?")).toBeVisible();
  await expectNoChinese(page, "结束确认");
  await button(page, "End interview").click();
  await expect(page.getByRole("heading", { name: "Interview ended" })).toBeVisible({
    timeout: 30_000,
  });
  await expectNoChinese(page, "结束页");

  const state = await expectRecordingsReady(interview, 1);
  expect(state.end_reason).toBe("candidate_left");
  expect(answerTo(state, "q1")?.end_reason).toBe("button");
});

test.describe("浏览器是英文的", () => {
  test.use({ locale: "en-US" });

  test("链接无效时还不知道是哪场面试，按浏览器语言显示；中文面试打开后还是中文", async ({
    page,
  }) => {
    await open(page, UNKNOWN_TOKEN);
    await expect(page.getByRole("heading", { name: "Invalid link" })).toBeVisible();
    await expectNoChinese(page, "链接无效页");

    const interview = seedInterview();
    await open(page, interview.token);
    await page.reload();
    await expect(button(page, "开始准备")).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.lang)).toBe("zh-CN");
  });
});
