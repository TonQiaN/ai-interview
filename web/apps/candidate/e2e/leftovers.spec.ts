import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  expectQuestion,
  LOCAL_DB,
  localRecordings,
  open,
  revokeInvites,
  seedInterview,
  sessionKeyOf,
  UNKNOWN_TOKEN,
} from "./helpers";

/**
 * 传不上去的录像不留在候选人的设备上。
 *
 * 录像分片先存在浏览器里，传上去一片删一片。链接失效以后（过期、被撤销、面试被取消或删除）
 * 服务端不再收，剩下的分片就得从本机删掉：里面是候选人的脸和声音，设备可能是借来的。
 */

const DAY = 24 * 60 * 60 * 1000;

/** 往浏览器里放一段别的时候留下的录像：一条段的登记加两片内容（几个字节的假数据）。 */
const leaveRecording = (page: Page, sessionKey: string, createdAt: number): Promise<void> =>
  page.evaluate(
    async ({ name, sessionKey, createdAt }) => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const transaction = db.transaction(["segments", "chunks", "chunk-index"], "readwrite");
      const segmentId = crypto.randomUUID();
      transaction.objectStore("segments").put({
        sessionKey,
        track: "camera",
        segmentId,
        mimeType: "video/webm",
        startMs: 0,
        totalChunks: null,
        createdAt,
      });
      for (const seq of [0, 1]) {
        transaction
          .objectStore("chunks")
          .put({ sessionKey, segmentId, seq, bytes: new ArrayBuffer(8), sha256: "x" });
        transaction.objectStore("chunk-index").put({ sessionKey, segmentId, seq, size: 8 });
      }
      await new Promise<void>((resolve, reject) => {
        transaction.oncomplete = () => resolve();
        transaction.onerror = () => reject(transaction.error);
      });
      db.close();
    },
    { name: LOCAL_DB, sessionKey, createdAt },
  );

test("面试到一半链接被撤销：告诉候选人面试中止了，没传上去的录像从本机删掉", async ({ page }) => {
  // 静音 30 秒才算答完：题目不会自己往下走
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const key = await sessionKeyOf(page, interview.token);

  // 录像传不上去，分片在本机越攒越多
  await page.route("**/api/v1/room/recordings/**", (route) => route.abort());
  await expect
    .poll(async () => (await localRecordings(page))?.[key] ?? 0, { timeout: 20_000 })
    .toBeGreaterThan(2);

  revokeInvites(interview);
  await page.unroute("**/api/v1/room/recordings/**");
  await button(page, "我答完了，下一题").click();

  await expect(page.getByRole("heading", { name: "面试已中止" })).toBeVisible({
    timeout: 30_000,
  });
  // 不让候选人对着一个传不上去的进度一直点重试
  await expect(button(page, "重试")).toHaveCount(0);
  await expect.poll(() => localRecordings(page)).toEqual({});
});

test("打开失效的链接时，把它留在本机的录像删掉；别的面试留下的只删放了很久的", async ({ page }) => {
  const interview = seedInterview();
  // 先让页面把本地的库建好
  await open(page, UNKNOWN_TOKEN);
  await expect(page.getByRole("heading", { name: "链接无效" })).toBeVisible();
  await expect.poll(() => localRecordings(page)).toEqual({});

  const now = Date.now();
  const revoked = await sessionKeyOf(page, interview.revoked_token);
  await leaveRecording(page, revoked, now - 60_000);
  await leaveRecording(page, "another-interview-last-month", now - 30 * DAY);
  await leaveRecording(page, "another-interview-yesterday", now - DAY);
  expect(await localRecordings(page)).toEqual({
    [revoked]: 2,
    "another-interview-last-month": 2,
    "another-interview-yesterday": 2,
  });

  await open(page, interview.revoked_token);
  await page.reload();
  await expect(page.getByRole("heading", { name: "链接已失效" })).toBeVisible();
  // 这条链接的删了，上个月的删了；昨天那场也许还在补传期内，留着
  await expect.poll(() => localRecordings(page)).toEqual({ "another-interview-yesterday": 2 });
});

test("打开还能用的链接时，也顺手清掉别的面试留下的旧录像", async ({ page }) => {
  const interview = seedInterview();
  await open(page, interview.token);
  await expect(button(page, "开始准备")).toBeVisible();
  await expect.poll(() => localRecordings(page)).toEqual({});

  const now = Date.now();
  await leaveRecording(page, "another-interview-last-month", now - 30 * DAY);
  await leaveRecording(page, "another-interview-yesterday", now - DAY);
  await page.reload();
  await expect(button(page, "开始准备")).toBeVisible();

  await expect.poll(() => localRecordings(page)).toEqual({ "another-interview-yesterday": 2 });
});
