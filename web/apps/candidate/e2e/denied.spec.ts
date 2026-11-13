import { expect, test } from "@playwright/test";
import { button, open, roomState, seedInterview } from "./helpers";

test("拒绝摄像头和麦克风权限：说明怎么改，可以重新检测，不能跳过", async ({ page }) => {
  const interview = seedInterview();
  await open(page, interview.token);
  await button(page, "开始准备").click();
  for (const box of await page.getByRole("checkbox").all()) await box.check();
  await button(page, "同意并继续").click();

  await button(page, "允许使用摄像头和麦克风").click();
  await expect(page.getByText("没有拿到摄像头或麦克风的权限")).toBeVisible();
  await expect(page.getByText("改好之后点下面的「重新检测」。")).toBeVisible();
  await expect(button(page, "下一步")).toHaveCount(0);

  await button(page, "重新检测").click();
  await expect(page.getByText("没有拿到摄像头或麦克风的权限")).toBeVisible();

  const state = roomState(interview);
  expect(state.status).toBe("invited");
  expect(state.recordings).toHaveLength(0);
  // 失败的原因会报给后端，HR 能看到候选人卡在了哪一步
  await expect
    .poll(() =>
      roomState(interview)
        .events.filter((event) => event.kind === "device_check")
        .map((event) => event.payload),
    )
    .toContainEqual({ result: "failed", error: "permission_denied" });
});
