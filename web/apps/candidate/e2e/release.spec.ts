import { expect, test } from "@playwright/test";
import {
  button,
  enterInterview,
  expectQuestion,
  liveTracks,
  open,
  passPreflight,
  revokeInvites,
  seedInterview,
  watchStreams,
} from "./helpers";

/**
 * 页面不再需要摄像头和麦克风的时候要把它们关掉：面试结束了、链接不能用了，
 * 手机上的摄像头指示灯还亮着，候选人会以为还在被录。
 */

test("提前结束面试：到了结束页，摄像头和麦克风都关了", async ({ page }) => {
  await watchStreams(page);
  await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  // 面试中画面和声音各一路
  expect(await liveTracks(page)).toBe(2);

  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });
  expect(await liveTracks(page)).toBe(0);
});

test("设备检查完、还没开始，链接被撤销：点开始后看到链接失效，摄像头和麦克风都关了", async ({
  page,
}) => {
  await watchStreams(page);
  const interview = seedInterview();
  await open(page, interview.token);
  await passPreflight(page);
  await expect(button(page, "开始面试")).toBeVisible();
  expect(await liveTracks(page)).toBe(2);

  revokeInvites(interview);
  await button(page, "开始面试").click();
  await expect(page.getByRole("heading", { name: "链接已失效" })).toBeVisible();
  await expect.poll(() => liveTracks(page)).toBe(0);
});

test("检查设备时麦克风断了：还开着的摄像头也关掉；重新检测后走完面试，没有哪一路还开着", async ({
  page,
}) => {
  await watchStreams(page);
  const interview = seedInterview({ silenceSec: 30 });
  await open(page, interview.token);
  await passPreflight(page);
  await expect(button(page, "开始面试")).toBeVisible();

  // 像耳机被拔掉：浏览器说麦克风这一路结束了
  await page.evaluate(() => {
    const scope = window as unknown as { __streams: MediaStream[] };
    for (const stream of scope.__streams) {
      for (const track of stream.getAudioTracks()) track.dispatchEvent(new Event("ended"));
    }
  });
  await expect(page.getByText("没有找到摄像头或麦克风")).toBeVisible();
  expect(await liveTracks(page)).toBe(0);

  await button(page, "重新检测").click();
  await button(page, "播放提示音").click();
  await button(page, "听到了").click();
  await button(page, "下一步").click();
  await button(page, "开始面试").click();
  await expect(page.getByText("录制中")).toBeVisible();
  // 只有重新申请到的这一次开着，上一次的没有留下
  expect(await liveTracks(page)).toBe(2);

  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });
  expect(await liveTracks(page)).toBe(0);
});
