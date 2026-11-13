/**
 * 面试时间轴：相对面试开始的毫秒数。
 *
 * 一场面试里所有时间点（每题的起止、过程事件、每段录像的起点）都用这条时间轴，
 * 报告里的证据靠它跳到录像的对应位置。见 docs/architecture/data-model.md 的「时间轴」。
 *
 * 用单调时钟（performance.now）计时，不受候选人改系统时间影响。
 * 页面刷新后单调时钟会归零，所以每次拿到服务端的「已经过去多少毫秒」都重新对一次。
 */
export class Timeline {
  /** 时间轴零点对应的单调时钟读数。 */
  private zero: number;

  /**
   * @param elapsedMs 此刻面试已经进行了多少毫秒。刚点「开始」时是 0；
   *                  续面时用服务端返回的值。
   * @param monotonicNow 单调时钟，毫秒。
   */
  constructor(
    elapsedMs = 0,
    private readonly monotonicNow: () => number = () => performance.now(),
  ) {
    this.zero = this.monotonicNow() - elapsedMs;
  }

  /** 时间轴上的当前时刻，整数毫秒，不会小于 0。 */
  now(): number {
    return Math.max(0, Math.round(this.monotonicNow() - this.zero));
  }

  /** 把一个单调时钟读数换算到时间轴上。 */
  at(monotonicMs: number): number {
    return Math.max(0, Math.round(monotonicMs - this.zero));
  }

  /** 用服务端的读数重新对时。 */
  resync(elapsedMs: number): void {
    this.zero = this.monotonicNow() - elapsedMs;
  }
}
