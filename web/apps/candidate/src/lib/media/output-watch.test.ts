import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type OutputHalt, OutputWatch, type WatchedContext } from "./output-watch";

/** 假的 AudioContext：running 时时钟跟着（假的）系统时间走，除非被卡住。 */
class FakeContext implements WatchedContext {
  state = "running";
  /** 状态写着 running 但时钟不走。 */
  stuck = false;
  /** resume 的表现：照做、报错、没反应。 */
  resumeMode: "ok" | "reject" | "hang" = "ok";
  resumeCalls = 0;
  suspendCalls = 0;
  private time = 0;
  private since = Date.now();
  private readonly listeners = new Set<() => void>();

  get currentTime(): number {
    if (this.state !== "running" || this.stuck) return this.time;
    return this.time + (Date.now() - this.since) / 1000;
  }

  /** 系统改了状态（来电话、切到后台）。 */
  setState(state: string): void {
    this.time = this.currentTime;
    this.since = Date.now();
    this.state = state;
    for (const listener of this.listeners) listener();
  }

  resume = async (): Promise<void> => {
    this.resumeCalls += 1;
    if (this.resumeMode === "reject") throw new Error("不让恢复");
    if (this.resumeMode === "hang") return new Promise(() => {});
    this.setState("running");
  };

  suspend = async (): Promise<void> => {
    this.suspendCalls += 1;
    this.stuck = false; // 停一下再开，时钟就走了
    this.setState("suspended");
  };

  addEventListener(_type: "statechange", listener: () => void): void {
    this.listeners.add(listener);
  }

  removeEventListener(_type: "statechange", listener: () => void): void {
    this.listeners.delete(listener);
  }
}

function setup() {
  const context = new FakeContext();
  const changes: Array<OutputHalt | null> = [];
  const watch = new OutputWatch({ context, onChange: (halt) => changes.push(halt) });
  watch.start();
  return { context, changes, watch };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("盯着音频是不是还在运转", () => {
  it("一直在转就什么都不报", async () => {
    const { changes, watch } = setup();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(changes).toEqual([]);
    expect(watch.halt).toBeNull();
  });

  it("被系统停掉马上报，自己恢复了也报", async () => {
    const { context, changes } = setup();
    await vi.advanceTimersByTimeAsync(5_000);

    context.setState("interrupted");
    expect(changes).toEqual(["interrupted"]);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(changes).toEqual(["interrupted"]);

    context.setState("running"); // 电话挂了，系统自己恢复
    expect(changes).toEqual(["interrupted"]); // 时钟走了才算
    await vi.advanceTimersByTimeAsync(1_000);
    expect(changes).toEqual(["interrupted", null]);
  });

  it("状态正常但时钟不走，连着三次才算停了", async () => {
    const { context, changes } = setup();
    context.stuck = true;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(changes).toEqual([]);
    context.stuck = false; // 只是卡了一下
    await vi.advanceTimersByTimeAsync(5_000);
    expect(changes).toEqual([]);

    context.stuck = true;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(changes).toEqual(["stalled"]);
  });

  it("恢复成功：时钟走了才算数", async () => {
    const { context, changes, watch } = setup();
    context.setState("suspended");

    const result = watch.revive();
    await vi.advanceTimersByTimeAsync(300);
    expect(await result).toBe(true);
    expect(context.resumeCalls).toBe(1);
    expect(changes).toEqual(["suspended", null]);
  });

  it("时钟不走的，先停再开", async () => {
    const { context, changes, watch } = setup();
    context.stuck = true;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(changes).toEqual(["stalled"]);

    const result = watch.revive();
    await vi.advanceTimersByTimeAsync(300);
    expect(await result).toBe(true);
    expect(context.suspendCalls).toBe(1);
    expect(changes).toEqual(["stalled", null]);
  });

  it("浏览器不让恢复：报错和没反应都算没成功，不会一直等", async () => {
    const { context, changes, watch } = setup();
    context.setState("interrupted");

    context.resumeMode = "reject";
    let result = watch.revive();
    await vi.advanceTimersByTimeAsync(300);
    expect(await result).toBe(false);

    context.resumeMode = "hang";
    result = watch.revive();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(await result).toBe(false);
    expect(changes).toEqual(["interrupted"]);

    context.resumeMode = "ok"; // 候选人点了按钮
    result = watch.revive();
    await vi.advanceTimersByTimeAsync(300);
    expect(await result).toBe(true);
    expect(changes).toEqual(["interrupted", null]);
  });

  it("同时调两次只恢复一次", async () => {
    const { context, watch } = setup();
    context.setState("suspended");
    const first = watch.revive();
    const second = watch.revive();
    await vi.advanceTimersByTimeAsync(300);
    expect(await first).toBe(true);
    expect(await second).toBe(true);
    expect(context.resumeCalls).toBe(1);
  });

  it("stop 之后不再报", async () => {
    const { context, changes, watch } = setup();
    watch.stop();
    context.setState("suspended");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(changes).toEqual([]);
  });
});
