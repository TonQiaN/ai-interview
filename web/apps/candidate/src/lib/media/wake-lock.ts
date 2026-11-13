/**
 * 面试中不让屏幕自己熄灭。
 *
 * 候选人听题的时候不碰屏幕，手机过一会儿就会自动锁屏，锁屏后摄像头和录制都会停。
 * 浏览器的 Screen Wake Lock 可以拦住自动熄屏，但有两个脾气：只在页面可见时给，
 * 页面切到后台它会自己松开。所以回到前台时要重新申请一次。
 *
 * 申请不到（浏览器不支持、省电模式）不影响面试，只是屏幕可能会自己熄灭。
 */

export interface WakeLockSentinelLike {
  release(): Promise<void>;
  addEventListener(type: "release", listener: () => void): void;
}

export interface WakeLockSource {
  request(type: "screen"): Promise<WakeLockSentinelLike>;
}

export interface VisibilitySource {
  visibilityState: string;
  addEventListener(type: "visibilitychange", listener: () => void): void;
  removeEventListener(type: "visibilitychange", listener: () => void): void;
}

export class ScreenWakeLock {
  private sentinel: WakeLockSentinelLike | null = null;
  private wanted = false;
  private requesting: Promise<boolean> | null = null;

  /** source 是 navigator.wakeLock，浏览器不支持时是 undefined。 */
  constructor(
    private readonly source: WakeLockSource | undefined,
    private readonly page: VisibilitySource,
  ) {}

  get supported(): boolean {
    return this.source !== undefined;
  }

  /** 现在有没有拦着屏幕熄灭。 */
  get held(): boolean {
    return this.sentinel !== null;
  }

  /** 开始保持屏幕常亮。返回这一次有没有申请到；没申请到的话，页面下次回到前台时还会再试。 */
  async acquire(): Promise<boolean> {
    if (!this.wanted) {
      this.wanted = true;
      this.page.addEventListener("visibilitychange", this.onVisibilityChange);
    }
    return this.request();
  }

  /** 不再保持。 */
  async release(): Promise<void> {
    if (this.wanted) {
      this.wanted = false;
      this.page.removeEventListener("visibilitychange", this.onVisibilityChange);
    }
    // 正在申请的那一次回来后会看到 wanted 已经是 false，自己松开
    const sentinel = this.sentinel;
    this.sentinel = null;
    await sentinel?.release().catch(() => {});
  }

  private request(): Promise<boolean> {
    if (this.sentinel) return Promise.resolve(true);
    const source = this.source;
    if (!source || this.page.visibilityState !== "visible") return Promise.resolve(false);
    this.requesting ??= this.requestOnce(source).finally(() => {
      this.requesting = null;
    });
    return this.requesting;
  }

  private async requestOnce(source: WakeLockSource): Promise<boolean> {
    let sentinel: WakeLockSentinelLike;
    try {
      sentinel = await source.request("screen");
    } catch {
      // 省电模式、页面刚好切到后台等情况下浏览器会拒绝
      return false;
    }
    if (!this.wanted) {
      await sentinel.release().catch(() => {});
      return false;
    }
    this.sentinel = sentinel;
    sentinel.addEventListener("release", () => {
      if (this.sentinel === sentinel) this.sentinel = null;
    });
    return true;
  }

  private onVisibilityChange = (): void => {
    if (this.wanted && this.page.visibilityState === "visible") void this.request();
  };
}
