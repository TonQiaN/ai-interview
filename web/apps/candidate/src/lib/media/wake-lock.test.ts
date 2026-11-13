import { describe, expect, it } from "vitest";
import { ScreenWakeLock, type WakeLockSentinelLike } from "./wake-lock";

class FakeSentinel implements WakeLockSentinelLike {
  released = false;
  private listeners: Array<() => void> = [];

  async release(): Promise<void> {
    if (this.released) return;
    this.released = true;
    for (const listener of this.listeners) listener();
  }

  addEventListener(_type: "release", listener: () => void): void {
    this.listeners.push(listener);
  }
}

class FakePage {
  visibilityState = "visible";
  private listeners = new Set<() => void>();

  addEventListener(_type: "visibilitychange", listener: () => void): void {
    this.listeners.add(listener);
  }

  removeEventListener(_type: "visibilitychange", listener: () => void): void {
    this.listeners.delete(listener);
  }

  get listening(): boolean {
    return this.listeners.size > 0;
  }

  /** 页面切到后台：浏览器会自己松开常亮。 */
  hide(sentinels: FakeSentinel[]): void {
    this.visibilityState = "hidden";
    for (const sentinel of sentinels) void sentinel.release();
    for (const listener of this.listeners) listener();
  }

  show(): void {
    this.visibilityState = "visible";
    for (const listener of this.listeners) listener();
  }
}

function fakeSource() {
  const sentinels: FakeSentinel[] = [];
  let refuse = false;
  return {
    sentinels,
    refuseNext: () => {
      refuse = true;
    },
    request: async (_type: "screen") => {
      if (refuse) {
        refuse = false;
        throw new DOMException("省电模式", "NotAllowedError");
      }
      const sentinel = new FakeSentinel();
      sentinels.push(sentinel);
      return sentinel;
    },
  };
}

/** 等已经排好的微任务跑完。 */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("ScreenWakeLock", () => {
  it("申请到以后一直拿着，松开时还给浏览器", async () => {
    const source = fakeSource();
    const page = new FakePage();
    const lock = new ScreenWakeLock(source, page);

    expect(await lock.acquire()).toBe(true);
    expect(lock.held).toBe(true);
    // 再申请一次不会多拿一把
    expect(await lock.acquire()).toBe(true);
    expect(source.sentinels).toHaveLength(1);

    await lock.release();
    expect(lock.held).toBe(false);
    expect(source.sentinels[0]?.released).toBe(true);
    expect(page.listening).toBe(false);
  });

  it("页面切到后台再回来，会重新申请", async () => {
    const source = fakeSource();
    const page = new FakePage();
    const lock = new ScreenWakeLock(source, page);
    await lock.acquire();

    page.hide(source.sentinels);
    await settle();
    expect(lock.held).toBe(false);
    expect(source.sentinels).toHaveLength(1);

    page.show();
    await settle();
    expect(lock.held).toBe(true);
    expect(source.sentinels).toHaveLength(2);
  });

  it("松开以后回到前台不再申请", async () => {
    const source = fakeSource();
    const page = new FakePage();
    const lock = new ScreenWakeLock(source, page);
    await lock.acquire();
    await lock.release();

    page.hide(source.sentinels);
    page.show();
    await settle();
    expect(source.sentinels).toHaveLength(1);
  });

  it("浏览器拒绝时不报错，下次回到前台再试", async () => {
    const source = fakeSource();
    const page = new FakePage();
    const lock = new ScreenWakeLock(source, page);

    source.refuseNext();
    expect(await lock.acquire()).toBe(false);
    expect(lock.held).toBe(false);

    page.hide(source.sentinels);
    page.show();
    await settle();
    expect(lock.held).toBe(true);
  });

  it("浏览器不支持时什么都不做", async () => {
    const page = new FakePage();
    const lock = new ScreenWakeLock(undefined, page);
    expect(lock.supported).toBe(false);
    expect(await lock.acquire()).toBe(false);
    await lock.release();
    expect(page.listening).toBe(false);
  });

  it("页面在后台时不去申请", async () => {
    const source = fakeSource();
    const page = new FakePage();
    page.visibilityState = "hidden";
    const lock = new ScreenWakeLock(source, page);
    expect(await lock.acquire()).toBe(false);
    expect(source.sentinels).toHaveLength(0);
  });

  it("申请还没回来就松开，回来的那一把马上还掉", async () => {
    const page = new FakePage();
    const sentinel = new FakeSentinel();
    let grant: (value: FakeSentinel) => void = () => {};
    const lock = new ScreenWakeLock(
      {
        request: () =>
          new Promise<FakeSentinel>((resolve) => {
            grant = resolve;
          }),
      },
      page,
    );

    const pending = lock.acquire();
    await lock.release();
    grant(sentinel);
    expect(await pending).toBe(false);
    expect(sentinel.released).toBe(true);
    expect(lock.held).toBe(false);
  });
});
