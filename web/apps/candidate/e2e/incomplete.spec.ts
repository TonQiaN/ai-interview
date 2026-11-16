import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  expectQuestion,
  expectRecordingsReady,
  LOCAL_DB,
  localRecordings,
  open,
  roomState,
  sessionKeyOf,
  walkAway,
} from "./helpers";

/**
 * 录像没能传完整的时候，HR 照样看得到已经传上去的部分，候选人也不会对着一个传不完的进度干等。
 *
 * 两种情况：候选人没等传完就关了页面；本机有一片没存下来，怎么重试都补不上。
 *
 * 最后一条是防着第二种情况发生：本机存不下的时候，分片先留在内存里接着传，录像是完整的。
 */

const UPLOADS = "**/api/v1/room/recordings/**";

/** 这场面试留在本机还没传上去的分片有几片。 */
const pendingChunks = async (page: Page, key: string): Promise<number> =>
  (await localRecordings(page))?.[key] ?? 0;

/** 把本机还没传的分片删掉一片（留着最前面的一片，删它后面那一片），返回删的是第几片。 */
const loseOneChunk = (page: Page, sessionKey: string): Promise<number> =>
  page.evaluate(
    async ({ name, sessionKey }) => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const transaction = db.transaction(["chunks", "chunk-index"], "readwrite");
      const index = transaction.objectStore("chunk-index");
      const all = await new Promise<{ sessionKey: string; segmentId: string; seq: number }[]>(
        (resolve, reject) => {
          const request = index.getAll();
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        },
      );
      const mine = all
        .filter((chunk) => chunk.sessionKey === sessionKey)
        .sort((a, b) => a.seq - b.seq);
      const lost = mine[1];
      if (!lost) throw new Error("本机的分片不够两片");
      const key = [lost.sessionKey, lost.segmentId, lost.seq];
      transaction.objectStore("chunks").delete(key);
      index.delete(key);
      await new Promise<void>((resolve, reject) => {
        transaction.oncomplete = () => resolve();
        transaction.onerror = () => reject(transaction.error);
      });
      db.close();
      return lost.seq;
    },
    { name: LOCAL_DB, sessionKey },
  );

test("候选人没等录像传完就关了页面：过一阵 HR 能看到传上去的部分；候选人再回来，本机剩下的删掉", async ({
  page,
}) => {
  // 静音 30 秒才算答完：题目不会自己往下走
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const key = await sessionKeyOf(page, interview.token);

  // 先传上去几片，之后的传不上去，在本机攒着
  await expect
    .poll(() => roomState(interview).recordings[0]?.chunks.length ?? 0, { timeout: 20_000 })
    .toBeGreaterThan(2);
  await page.route(UPLOADS, (route) => route.abort());
  await expect.poll(() => pendingChunks(page, key), { timeout: 20_000 }).toBeGreaterThan(2);

  // 关了页面，没有再回来
  await page.goto("about:blank");
  await page.unroute(UPLOADS);
  const uploaded = roomState(interview).recordings[0]?.chunks.length ?? 0;
  expect(roomState(interview).recordings[0]?.status).toBe("uploading");

  // 45 分钟过去了：服务端不再等，把收到的部分合并出来
  walkAway(interview, 45);
  await expect
    .poll(() => roomState(interview).recordings.map((recording) => recording.status), {
      timeout: 30_000,
    })
    .toEqual(["ready"]);
  const [recording] = roomState(interview).recordings;
  expect(recording?.total_chunks).toBe(uploaded);
  expect(recording?.size_bytes).toBeGreaterThan(0);
  expect(recording?.error).toContain("没有传完");

  // 候选人这时候才想起来，又打开了链接：剩下的传不进去了，不让他对着进度一直等
  await open(page, interview.token);
  await expect(page.getByText("面试已中断")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => localRecordings(page), { timeout: 30_000 }).toEqual({});
  await expect(button(page, "重试")).toHaveCount(0);
  expect(roomState(interview).recordings.map((item) => item.status)).toEqual(["ready"]);
});

test("本机有一片录像没存下来：面试照常结束，不卡在重试，HR 能看到缺口之前的部分", async ({
  page,
}) => {
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const key = await sessionKeyOf(page, interview.token);

  await page.route(UPLOADS, (route) => route.abort());
  await expect.poll(() => pendingChunks(page, key), { timeout: 20_000 }).toBeGreaterThan(3);
  const lost = await loseOneChunk(page, key);
  await page.unroute(UPLOADS);

  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  // 缺的那一片谁也补不上：服务端把它之前的合并出来，页面这边传完收工
  await expect
    .poll(() => roomState(interview).recordings.map((recording) => recording.status), {
      timeout: 60_000,
    })
    .toEqual(["ready"]);
  const [recording] = roomState(interview).recordings;
  expect(recording?.total_chunks).toBe(lost);
  expect(recording?.error).toContain("中间缺了");
  await expect.poll(() => localRecordings(page), { timeout: 30_000 }).toEqual({});
  await expect(button(page, "重试")).toHaveCount(0);
});

test("面试到一半本机存不下录像了：分片留在内存里接着传，录像是完整的", async ({ page }) => {
  // 像磁盘满了那样：开关打开以后，往本机的库里写什么都失败（读和删照常）
  await page.addInitScript(() => {
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (this: IDBObjectStore, ...args) {
      if ((window as unknown as { __diskFull?: boolean }).__diskFull) {
        throw new DOMException("磁盘满了", "QuotaExceededError");
      }
      return put.apply(this, args);
    };
  });
  const interview = await enterInterview(page, { silenceSec: 30 });
  await expectQuestion(page, 1);
  const uploaded = () => roomState(interview).recordings[0]?.chunks.length ?? 0;

  await expect.poll(uploaded, { timeout: 20_000 }).toBeGreaterThan(1);
  await page.evaluate(() => Object.assign(window, { __diskFull: true }));
  const before = uploaded();

  // 存不下以后录的几片照样传上去了；一直到面试结束都存不下
  await expect.poll(uploaded, { timeout: 20_000 }).toBeGreaterThan(before + 3);
  await button(page, "结束").click();
  await button(page, "结束面试").click();
  await expect(page.getByText("面试已结束")).toBeVisible({ timeout: 30_000 });

  const state = await expectRecordingsReady(interview, 1);
  expect(state.recordings[0]?.error).toBeNull();
  expect(state.recordings[0]?.total_chunks).toBeGreaterThan(before + 3);
  // 这件事记了一条过程事件，只记一次
  const errors = state.events.filter((event) => event.kind === "recording_error");
  expect(errors.map((event) => event.payload)).toEqual([
    { message: "QuotaExceededError", kept_in_memory: true },
  ]);
  await expect(button(page, "重试")).toHaveCount(0);
});
