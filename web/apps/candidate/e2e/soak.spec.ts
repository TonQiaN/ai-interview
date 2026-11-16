import { execFileSync } from "node:child_process";
import { expect, test } from "@playwright/test";
import { enterInterview, localRecordings, roomState, sessionKeyOf } from "./helpers";

/**
 * 长时间面试：真的面试有二三十分钟，平时的浏览器测试每场只有十几秒，看不出慢慢攒起来的问题
 * （内存越用越多、本机的录像越积越多、录像合并出来时长对不上）。
 *
 *   E2E_SOAK_MINUTES=15 pnpm e2e soak
 *
 * 不设这个变量就跳过，CI 不跑。假麦克风每 5 秒里说 2.5 秒，静音 30 秒才算答完，
 * 所以每次作答都答到最长作答时间；三道题加一次追问，四次加起来大约就是设的分钟数。
 * 每半分钟打印一行：页面用了多少内存、本机还留着几片、服务端收到了几片。
 */
const MINUTES = Number(process.env.E2E_SOAK_MINUTES ?? 0);
/** 候选人作答几次：三道题，第一题有一次追问 */
const ANSWERS = 4;

interface Sample {
  minute: number;
  heapMb: number;
  nodes: number;
  listeners: number;
  local: number;
  received: number;
}

test("面试十几分钟：内存不涨、本机的录像不积压、合并出来的录像和面试一样长", async ({ page }) => {
  test.skip(MINUTES <= 0, "设了 E2E_SOAK_MINUTES 才跑");
  test.setTimeout((MINUTES * 2 + 5) * 60_000);
  const maxSec = Math.min(600, Math.max(40, Math.round((MINUTES * 60) / ANSWERS)));

  const interview = await enterInterview(page, { silenceSec: 30, maxSec });
  const startedAt = Date.now();
  const sessionKey = await sessionKeyOf(page, interview.token);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");

  const sample = async (): Promise<Sample> => {
    // 先回收一遍，量到的才是真的占着的
    await cdp.send("HeapProfiler.collectGarbage");
    const { metrics } = await cdp.send("Performance.getMetrics");
    const metric = (name: string) => metrics.find((item) => item.name === name)?.value ?? 0;
    const local = (await localRecordings(page))?.[sessionKey] ?? 0;
    const received = roomState(interview).recordings.reduce(
      (sum, recording) => sum + recording.chunks.length,
      0,
    );
    return {
      minute: Math.round((Date.now() - startedAt) / 6000) / 10,
      heapMb: Math.round((metric("JSHeapUsedSize") / 1024 / 1024) * 10) / 10,
      nodes: metric("Nodes"),
      listeners: metric("JSEventListeners"),
      local,
      received,
    };
  };

  const samples: Sample[] = [];
  const done = page.getByText("面试完成，谢谢你");
  while (!(await done.isVisible())) {
    const now = await sample();
    samples.push(now);
    console.log(
      `${now.minute.toFixed(1)} 分钟  内存 ${now.heapMb} MB  节点 ${now.nodes}  监听 ${now.listeners}  本机 ${now.local} 片  服务端 ${now.received} 片`,
    );
    if (Date.now() - startedAt > (MINUTES * 2 + 3) * 60_000) throw new Error("面试一直没有结束");
    await done.waitFor({ timeout: 30_000 }).catch(() => undefined);
  }
  const elapsedMs = Date.now() - startedAt;

  // 录像一直传得上去：本机留着的始终只有刚录下的几片
  expect(Math.max(...samples.map((item) => item.local))).toBeLessThan(10);
  // 内存：后一半时间里最多的时候，比前一半最多的时候多不出一半
  const half = Math.floor(samples.length / 2);
  const peak = (items: Sample[]) => Math.max(...items.map((item) => item.heapMb));
  expect(samples.length).toBeGreaterThan(4);
  expect(peak(samples.slice(half))).toBeLessThan(peak(samples.slice(1, half)) * 1.5 + 5);

  // 合并出来是一整段，和面试一样长，播放器读得出时长
  await expect
    .poll(() => roomState(interview).recordings.map((recording) => recording.status), {
      timeout: 120_000,
    })
    .toEqual(["ready"]);
  const state = roomState(interview);
  const [recording] = state.recordings;
  expect(state.status).toBe("completed");
  expect(recording?.chunks.length).toBe(recording?.total_chunks);
  const durationMs = recording?.duration_ms ?? 0;
  expect(Math.abs(durationMs - elapsedMs)).toBeLessThan(15_000);
  const probed = Number(
    execFileSync(
      "ffprobe",
      ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", recording?.file ?? ""],
      { encoding: "utf8" },
    ).trim(),
  );
  expect(Math.abs(probed * 1000 - durationMs)).toBeLessThan(3000);
  console.log(
    `面试 ${(elapsedMs / 60_000).toFixed(1)} 分钟，录像 ${(durationMs / 60_000).toFixed(1)} 分钟、${((recording?.size_bytes ?? 0) / 1024 / 1024).toFixed(1)} MB、${recording?.total_chunks} 片；过程事件 ${state.events.length} 条`,
  );
});
