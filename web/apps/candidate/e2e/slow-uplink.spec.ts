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

/** 限速限到多少：录像码率的七分之一左右，半分钟的面试攒下的录像要传两三分钟。 */
const SLOW_KBPS = 48;
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

/**
 * 结束页上写的「还剩 1.4 MB，大约还要 3 分钟」：还剩多少字节、大约还要几分钟。
 * 这时候没显示就是 null；没写时间、或者写的是「不到一分钟」，分钟数是 null。
 */
async function shownRemaining(
  page: Page,
): Promise<{ bytes: number; minutes: number | null } | null> {
  const text = await page
    .getByText(/^还剩 /)
    .textContent({ timeout: 1000 })
    .catch(() => null);
  const match = text?.match(/^还剩 ([\d.]+) (KB|MB)(?:，大约还要 (\d+) 分钟|，不到一分钟)?$/);
  if (!match) return null;
  return {
    bytes: Number(match[1]) * (match[2] === "MB" ? 1024 * 1024 : 1024),
    minutes: match[3] === undefined ? null : Number(match[3]),
  };
}

/** 页面现在拦着屏幕熄灭的有几个（申请了还没松开的屏幕常亮）。 */
const screenLocksHeld = (page: Page) =>
  page.evaluate(
    () =>
      (window as unknown as { __screenLocks: { released: boolean }[] }).__screenLocks.filter(
        (lock) => !lock.released,
      ).length,
  );

test("上传比录像慢：面试照常往下走，结束页上「还剩多少」和本机真的留着的对得上，只减不增，还说了大约要等几分钟", async ({
  page,
}) => {
  // 没有界面的浏览器不给屏幕常亮，换一个假的：记下页面申请的每一个，后面看它什么时候松开
  await page.addInitScript(() => {
    const locks: { released: boolean }[] = [];
    Object.assign(window, { __screenLocks: locks });
    const wakeLock = {
      request: async () => {
        const lock = {
          released: false,
          release: async () => {
            lock.released = true;
          },
          addEventListener: () => {},
        };
        locks.push(lock);
        return lock;
      },
    };
    Object.defineProperty(Navigator.prototype, "wakeLock", { get: () => wakeLock });
  });
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
  // 录像大约每秒 40 KB，这里每秒只传得出去 6 KB
  await uplink(SLOW_KBPS);

  // 本机攒了几片之后提醒一句，面试不停
  await expect(page.getByText(SLOW_HINT)).toBeVisible({ timeout: 30_000 });
  await expectQuestion(page, 1);
  await expect(button(page, "我答完了，下一题")).toBeVisible({ timeout: 30_000 });
  await button(page, "我答完了，下一题").click();
  await expect(button(page, "我答完了，下一题")).toBeHidden();
  await expect(button(page, "我答完了，下一题")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByText("录制中")).toBeVisible();
  expect(await screenLocksHeld(page)).toBe(1);
  // 越攒越多的时候提醒一直在
  expect((await localBacklog(page, sessionKey)).chunks).toBeGreaterThan(10);
  await expect(page.getByText(SLOW_HINT)).toBeVisible();

  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试结束了，正在保存录像")).toBeVisible({ timeout: 30_000 });

  // 结束页上的数和本机真的留着的差不了多少（以前只算了手上这一轮开始时的那几片，不到一半）
  const settled: number[] = [];
  const waits: number[] = [];
  for (let second = 0; second < 8; second += 1) {
    const local = await localBacklog(page, sessionKey);
    const remaining = await shownRemaining(page);
    if (remaining !== null) {
      expect(remaining.bytes).toBeGreaterThan(local.bytes * 0.8);
      expect(remaining.bytes).toBeLessThan(local.bytes * 1.2 + 100 * 1024);
      // 最后一片落到本机之后不会再有新的：从这时候起这个数只减不增
      if (!local.recording) settled.push(remaining.bytes);
      // 说的时间和「剩下的除以限的速度」差不了太多
      if (remaining.minutes !== null) {
        const expected = local.bytes / (SLOW_KBPS * 125) / 60;
        expect(remaining.minutes).toBeGreaterThan(expected * 0.7);
        expect(remaining.minutes).toBeLessThan(expected * 2 + 1);
        waits.push(remaining.minutes);
      }
    }
    // 录像还没传完：手机这时候自己锁屏的话上传就停了，所以屏幕常亮要留到传完
    expect(await screenLocksHeld(page)).toBe(1);
    await page.waitForTimeout(1000);
  }
  expect(settled.length).toBeGreaterThan(3);
  // 要等好几分钟：写了大约多久，上面那句也不再说「一般不到一分钟」
  expect(waits.length).toBeGreaterThan(3);
  await expect(page.getByText(/你的网络上传比较慢/)).toBeVisible();
  await expect(page.getByText(/一般不到一分钟/)).toBeHidden();

  // 网络好了：剩下的很快传完，这期间的数也只减不增
  await uplink(0);
  const done = page.getByText("面试已结束");
  while (!(await done.isVisible())) {
    const remaining = await shownRemaining(page);
    if (remaining !== null) settled.push(remaining.bytes);
    await done.waitFor({ timeout: 500 }).catch(() => undefined);
  }
  for (let index = 1; index < settled.length; index += 1) {
    expect(settled[index]).toBeLessThanOrEqual(settled[index - 1] ?? 0);
  }
  await expectRecordingsReady(interview, 1);
  expect((await localBacklog(page, sessionKey)).chunks).toBe(0);
  // 传完了，不用再拦着屏幕熄灭
  await expect.poll(() => screenLocksHeld(page)).toBe(0);
});
