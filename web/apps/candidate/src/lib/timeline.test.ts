import { describe, expect, it } from "vitest";
import { Timeline } from "./timeline";

describe("Timeline", () => {
  it("从点「开始」那一刻起算", () => {
    let clock = 5_000;
    const timeline = new Timeline(0, () => clock);
    expect(timeline.now()).toBe(0);
    clock = 6_234.6;
    expect(timeline.now()).toBe(1_235);
  });

  it("续面：从服务端告知的已过时间接着算", () => {
    let clock = 100;
    const timeline = new Timeline(412_000, () => clock);
    expect(timeline.now()).toBe(412_000);
    clock = 1_100;
    expect(timeline.now()).toBe(413_000);
  });

  it("换算单调时钟读数，早于零点的按 0 算", () => {
    const timeline = new Timeline(0, () => 5_000);
    expect(timeline.at(7_500)).toBe(2_500);
    expect(timeline.at(4_000)).toBe(0);
  });

  it("重新对时", () => {
    let clock = 0;
    const timeline = new Timeline(0, () => clock);
    clock = 10_000;
    timeline.resync(9_400);
    expect(timeline.now()).toBe(9_400);
    clock = 10_600;
    expect(timeline.now()).toBe(10_000);
  });
});
