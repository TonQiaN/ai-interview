import { describe, expect, it, vi } from "vitest";
import { scheduleAll } from "./schedule";

describe("一次给几位候选人安排面试", () => {
  it("一个一个来，按选的顺序", async () => {
    const running: string[] = [];
    let most = 0;
    const outcome = await scheduleAll(["甲", "乙", "丙"], async (name) => {
      running.push(name);
      most = Math.max(most, running.length);
      await Promise.resolve();
      running.pop();
      return `面试-${name}`;
    });
    expect(most).toBe(1);
    expect(outcome.done.map((d) => d.result)).toEqual(["面试-甲", "面试-乙", "面试-丙"]);
    expect(outcome.failed).toEqual([]);
  });

  it("中间有人没成，后面的人照样安排，最后说清谁没成", async () => {
    const boom = new Error("这位候选人已经被删掉了");
    const outcome = await scheduleAll(["甲", "乙", "丙"], async (name) => {
      if (name === "乙") throw boom;
      return name;
    });
    expect(outcome.done.map((d) => d.person)).toEqual(["甲", "丙"]);
    expect(outcome.failed).toEqual([{ person: "乙", error: boom }]);
  });

  it("每安排完一位报一次进度", async () => {
    const progress = vi.fn();
    await scheduleAll(["甲", "乙"], async (name) => name, progress);
    expect(progress.mock.calls).toEqual([
      [1, 2],
      [2, 2],
    ]);
  });
});
