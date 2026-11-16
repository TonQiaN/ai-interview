import { expect, test } from "@playwright/test";
import { MIC_LOOP_MS } from "./fake-mic";
import {
  answerTo,
  button,
  enterInterview,
  eventKinds,
  expectQuestion,
  expectRecordingsReady,
  liveTracks,
  open,
  passPreflight,
  roomState,
  seedInterview,
  watchStreams,
} from "./helpers";

/**
 * 页面自己出问题的时候：脚本还没下完、没下下来、跑不起来，或者渲染到一半出错。
 * 这几种以前都是一片空白，候选人不知道是在加载还是坏了。
 */

/** 页面的入口脚本。开发服务器上是这个地址，构建之后是 assets/ 下带哈希的那个，都是同一个 module script。 */
const ENTRY = "**/src/main.tsx";
/** 脚本跑起来之前 index.html 里自带的那一块。页面一渲染出来它就没了。 */
const BOOT = "#boot";

test("脚本还没下完：先显示「正在打开面试」，等久了多说一句并给刷新按钮，下完后换成面试页", async ({
  page,
}) => {
  const interview = seedInterview();
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(ENTRY, async (route) => {
    await held;
    await route.continue();
  });
  // 入口脚本卡着的时候页面的 load 事件不会来，只等到文档开始加载
  await page.goto(`/i/#${interview.token}`, { waitUntil: "commit" });

  const boot = page.locator(BOOT);
  await expect(boot.getByText("正在打开面试")).toBeVisible();
  await expect(boot.getByRole("button", { name: "刷新页面" })).toBeHidden();

  // 等了八秒还没出来：说一句网络慢，给一个刷新按钮
  await expect(boot.getByText(/网络比较慢/)).toBeVisible({ timeout: 12_000 });
  await expect(boot.getByRole("button", { name: "刷新页面" })).toBeVisible();

  release();
  await expect(button(page, "开始准备")).toBeVisible();
  await expect(boot).toHaveCount(0);
});

test("脚本没下下来：告诉候选人检查网络后刷新，刷新后照常打开", async ({ page }) => {
  const interview = seedInterview();
  await page.route(ENTRY, (route) => route.abort("failed"));
  await page.goto(`/i/#${interview.token}`, { waitUntil: "commit" });

  const boot = page.locator(BOOT);
  await expect(boot.getByText("页面没有加载出来")).toBeVisible();
  await expect(boot.getByText(/请检查网络后刷新页面/)).toBeVisible();

  await page.unroute(ENTRY);
  await boot.getByRole("button", { name: "刷新页面" }).click();
  await expect(button(page, "开始准备")).toBeVisible();
});

test("脚本跑不起来（浏览器太旧，不认识里面的写法）：同样给出提示，不是一片空白", async ({
  page,
}) => {
  const interview = seedInterview();
  await page.route(ENTRY, (route) =>
    route.fulfill({ contentType: "text/javascript", body: "这不是这个浏览器认识的脚本 {" }),
  );
  await page.goto(`/i/#${interview.token}`, { waitUntil: "commit" });

  const boot = page.locator(BOOT);
  await expect(boot.getByText("页面没有加载出来")).toBeVisible();
  await expect(boot.getByText(/换用最新版的 Chrome、Edge 或 Safari/)).toBeVisible();
  await expect(boot.getByRole("button", { name: "刷新页面" })).toBeVisible();
});

test("浏览器是英文的：脚本没下下来时的提示是英文", async ({ browser }) => {
  const context = await browser.newContext({ locale: "en-US" });
  const page = await context.newPage();
  try {
    const interview = seedInterview();
    await page.route(ENTRY, (route) => route.abort("failed"));
    await page.goto(`/i/#${interview.token}`, { waitUntil: "commit" });
    const boot = page.locator(BOOT);
    await expect(boot.getByText("This page didn't load")).toBeVisible();
    await expect(boot.getByRole("button", { name: "Reload the page" })).toBeVisible();
  } finally {
    await context.close();
  }
});

test("链接是用 http 打开的：说明页面不是通过安全连接打开的，不是一片空白", async ({ page }) => {
  // 不是 https（也不是本机）的页面上，浏览器不给摄像头和麦克风，也没有 crypto.randomUUID 和 crypto.subtle。
  // 测试里的地址是本机，这里把这几样拿掉，装成那种页面
  await page.addInitScript(() => {
    Object.defineProperty(window, "isSecureContext", { value: false });
    Object.defineProperty(Navigator.prototype, "mediaDevices", { get: () => undefined });
    Object.defineProperty(Crypto.prototype, "randomUUID", { value: undefined });
    Object.defineProperty(Crypto.prototype, "subtle", { get: () => undefined });
  });
  const interview = seedInterview();
  await open(page, interview.token);
  await expect(page.getByRole("heading", { name: "这个浏览器不支持面试" })).toBeVisible();
  await expect(page.getByText("页面不是通过安全连接（https）打开的。")).toBeVisible();
});

test("页面一打开就出错（面试还没建起来）：显示「请刷新」，不是一片空白", async ({ page }) => {
  // 把页面一打开就要用的随机数拿掉，建面试会话的那一步会抛错
  await page.addInitScript(() => {
    Object.defineProperty(Crypto.prototype, "randomUUID", { value: undefined });
    Object.defineProperty(Crypto.prototype, "getRandomValues", { value: undefined });
  });
  const interview = seedInterview();
  await open(page, interview.token);
  await expect(page.getByRole("heading", { name: "页面出了点问题" })).toBeVisible();
  await expect(button(page, "刷新页面")).toBeVisible();
});

test("面试中页面渲染出错：不是一片空白，面试停住、摄像头和麦克风关掉，记一条过程事件，刷新后从中断的那一题继续", async ({
  page,
}) => {
  await watchStreams(page);
  const interview = await enterInterview(page);
  await expectQuestion(page, 1);

  // 下一步里混进页面画不出来的内容（该是文字的地方来了个对象），渲染时会抛错
  let advances = 0;
  await page.route("**/api/v1/room/advance", async (route) => {
    advances += 1;
    const response = await route.fetch();
    const step = await response.json();
    if (step.utterance) step.utterance.text = { unexpected: true };
    await route.fulfill({ response, json: step });
  });
  // 假麦克风说两秒半、停两秒半，第一题自己就答完了

  await expect(page.getByRole("heading", { name: "页面出了点问题" })).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByText(/已经录下和回答过的内容都保存着/)).toBeVisible();
  await expect
    .poll(() => roomState(interview).events.filter((event) => event.kind === "page_error"))
    .toEqual([expect.objectContaining({ payload: { stage: "interview", name: "Error" } })]);

  // 停住了：假麦克风又说完一轮，面试没有在候选人看不见的地方接着收第二题的回答
  await page.waitForTimeout(MIC_LOOP_MS * 2);
  expect(advances).toBe(1);
  expect(answerTo(roomState(interview), "q2")).toBeUndefined();

  // 候选人面前只剩「请刷新」：不再录了，摄像头的指示灯不该还亮着。这是页面自己关的，不算设备出了问题
  expect(await liveTracks(page)).toBe(0);
  expect(eventKinds(roomState(interview))).not.toContain("recording_gap");

  await page.unroute("**/api/v1/room/advance");
  // 页面自己让候选人刷新的，浏览器不该再弹一句「要离开吗，更改可能不会保存」
  const prompts: string[] = [];
  page.on("dialog", (dialog) => {
    prompts.push(dialog.type());
    void dialog.accept();
  });
  await button(page, "刷新页面").click();
  await expect(button(page, "检查设备，继续面试")).toBeVisible();
  expect(prompts).toEqual([]);
  await passPreflight(page, { resuming: true });
  await button(page, "继续面试").click();
  await expectQuestion(page, 2);

  // 出错之前录下的那一段收好了、传完了，和刷新之后的那一段都完整
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });
  await expectRecordingsReady(interview, 2);
});
