import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  expectQuestion,
  expectRecordingsReady,
  LOCAL_DB,
  sessionKeyOf,
} from "./helpers";

/**
 * 上传比录像慢的网络（弱的 Wi-Fi、信号不好的手机）：录下的分片在本机越攒越多。
 * 面试照常往下走；页面上说的「还剩多少没传」要和本机真的留着的对得上，不能传着传着又跳回去。
 */

const SLOW_HINT = "网络较慢，部分录像还在上传。面试可以正常进行。";

/** 本机这场面试还留着多少录像：几片、多少字节，还有没有正在录的段。 */
const localBacklog = (page: Page, sessionKey: string) =>
  page.evaluate(
    async ([name, key]) => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      try {
        const transaction = db.transaction(["segments", "chunk-index"], "readonly");
        const all = <T>(store: string) =>
          new Promise<T[]>((resolve, reject) => {
            const request = transaction.objectStore(store).getAll();
            request.onsuccess = () => resolve(request.result as T[]);
            request.onerror = () => reject(request.error);
          });
        const segments = (
          await all<{ sessionKey: string; totalChunks: number | null }>("segments")
        ).filter((row) => row.sessionKey === key);
        const chunks = (await all<{ sessionKey: string; size: number }>("chunk-index")).filter(
          (row) => row.sessionKey === key,
        );
        return {
          chunks: chunks.length,
          bytes: chunks.reduce((sum, row) => sum + row.size, 0),
          recording: segments.some((row) => row.totalChunks === null),
        };
      } finally {
        db.close();
      }
    },
    [LOCAL_DB, sessionKey] as const,
  );

/** 结束页上写的「还剩 1.4 MB」是多少字节；这时候没显示就是 null。 */
async function shownRemaining(page: Page): Promise<number | null> {
  const text = await page
    .getByText(/^还剩 /)
    .textContent({ timeout: 1000 })
    .catch(() => null);
  const match = text?.match(/^还剩 ([\d.]+) (KB|MB)$/);
  if (!match) return null;
  return Number(match[1]) * (match[2] === "MB" ? 1024 * 1024 : 1024);
}

test("上传比录像慢：面试照常往下走，结束页上「还剩多少」和本机真的留着的对得上，只减不增", async ({
  page,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  const sessionKey = await sessionKeyOf(page, interview.token);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Network.enable");
  const uplink = (kbps: number) =>
    cdp.send("Network.emulateNetworkConditions", {
      offline: false,
      latency: kbps > 0 ? 150 : 0,
      downloadThroughput: -1,
      uploadThroughput: kbps > 0 ? (kbps * 1000) / 8 : -1,
    });
  // 录像大约每秒 40 KB，这里每秒只传得出去 10 KB
  await uplink(80);

  // 本机攒了几片之后提醒一句，面试不停
  await expect(page.getByText(SLOW_HINT)).toBeVisible({ timeout: 30_000 });
  await expectQuestion(page, 1);
  await expect(button(page, "我答完了，下一题")).toBeVisible({ timeout: 30_000 });
  await button(page, "我答完了，下一题").click();
  await expect(button(page, "我答完了，下一题")).toBeHidden();
  await expect(button(page, "我答完了，下一题")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByText("录制中")).toBeVisible();
  // 越攒越多的时候提醒一直在
  expect((await localBacklog(page, sessionKey)).chunks).toBeGreaterThan(10);
  await expect(page.getByText(SLOW_HINT)).toBeVisible();

  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试结束了，正在保存录像")).toBeVisible({ timeout: 30_000 });

  // 结束页上的数和本机真的留着的差不了多少（以前只算了手上这一轮开始时的那几片，不到一半）
  const settled: number[] = [];
  for (let second = 0; second < 8; second += 1) {
    const local = await localBacklog(page, sessionKey);
    const remaining = await shownRemaining(page);
    if (remaining !== null) {
      expect(remaining).toBeGreaterThan(local.bytes * 0.8);
      expect(remaining).toBeLessThan(local.bytes * 1.2 + 100 * 1024);
      // 最后一片落到本机之后不会再有新的：从这时候起这个数只减不增
      if (!local.recording) settled.push(remaining);
    }
    await page.waitForTimeout(1000);
  }
  expect(settled.length).toBeGreaterThan(3);

  // 网络好了：剩下的很快传完，这期间的数也只减不增
  await uplink(0);
  const done = page.getByText("面试已结束");
  while (!(await done.isVisible())) {
    const remaining = await shownRemaining(page);
    if (remaining !== null) settled.push(remaining);
    await done.waitFor({ timeout: 500 }).catch(() => undefined);
  }
  for (let index = 1; index < settled.length; index += 1) {
    expect(settled[index]).toBeLessThanOrEqual(settled[index - 1] ?? 0);
  }
  await expectRecordingsReady(interview, 1);
  expect((await localBacklog(page, sessionKey)).chunks).toBe(0);
});
