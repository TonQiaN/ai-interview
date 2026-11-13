/**
 * 收集面试过程中页面上发生的事（切到别的标签页、粘贴、断网、换设备……），攒一批发给服务端。
 *
 * 这些事件是给分析报告用的线索，不是判定：切出页面可能只是接了个电话。
 * 所以只记事实和时间，不记内容（粘贴了什么、复制了什么一律不记）。
 *
 * 发不出去就留着下次再发；攒得太多时丢最旧的，不让内存涨上去。
 */
import type { ClientEvent } from "../api/room-types";

export interface EventCollectorOptions {
  send: (events: ClientEvent[], options: { keepalive: boolean }) => Promise<void>;
  /** 面试时间轴上的当前时刻；面试还没开始时返回 null。 */
  timelineNow: () => number | null;
  /** 当前的时刻，默认取系统时间。 */
  wallNow?: () => Date;
  /** 多久发一批（毫秒）。 */
  flushIntervalMs?: number;
  /** 一批最多多少条。 */
  maxBatch?: number;
  /** 最多攒多少条没发出去的。 */
  maxBuffered?: number;
}

export class EventCollector {
  private readonly send: EventCollectorOptions["send"];
  private readonly timelineNow: () => number | null;
  private readonly wallNow: () => Date;
  private readonly flushIntervalMs: number;
  private readonly maxBatch: number;
  private readonly maxBuffered: number;

  private buffer: ClientEvent[] = [];
  private timer: ReturnType<typeof setInterval> | null = null;
  private sending: Promise<void> | null = null;
  /** 因为攒得太多丢掉了几条。下次发送时报上去。 */
  private dropped = 0;

  constructor(options: EventCollectorOptions) {
    this.send = options.send;
    this.timelineNow = options.timelineNow;
    this.wallNow = options.wallNow ?? (() => new Date());
    this.flushIntervalMs = options.flushIntervalMs ?? 5_000;
    this.maxBatch = options.maxBatch ?? 50;
    this.maxBuffered = options.maxBuffered ?? 500;
  }

  get pending(): number {
    return this.buffer.length;
  }

  record(kind: string, payload: Record<string, unknown> = {}): void {
    this.buffer.push({ kind, at_ms: this.timelineNow(), occurred_at: this.wallNow().toISOString(), payload });
    if (this.buffer.length > this.maxBuffered) {
      const overflow = this.buffer.length - this.maxBuffered;
      this.buffer.splice(0, overflow);
      this.dropped += overflow;
    }
    if (this.buffer.length >= this.maxBatch) void this.flush();
  }

  start(): void {
    if (this.timer !== null) return;
    this.timer = setInterval(() => void this.flush(), this.flushIntervalMs);
  }

  stop(): void {
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
  }

  /**
   * 把攒着的事件发出去。失败不抛错，事件留着下次再发。
   * @param keepalive 页面要关了：让浏览器在页面关掉之后也把请求发完。
   */
  flush(options: { keepalive?: boolean } = {}): Promise<void> {
    const keepalive = options.keepalive ?? false;
    if (this.sending) {
      // 已经有一批在路上。页面要关时不能等，直接再发一批；平时等它回来再说
      if (!keepalive) return this.sending;
    }
    if (this.buffer.length === 0) return Promise.resolve();

    const batch = this.buffer.splice(0, this.maxBatch);
    if (this.dropped > 0) {
      batch.push({
        kind: "events_dropped",
        at_ms: this.timelineNow(),
        occurred_at: this.wallNow().toISOString(),
        payload: { count: this.dropped },
      });
      this.dropped = 0;
    }

    const attempt = this.send(batch, { keepalive }).then(
      () => {},
      () => {
        // 放回队首，保持时间顺序
        this.buffer.unshift(...batch.filter((event) => event.kind !== "events_dropped"));
      },
    );
    const tracked = attempt.finally(() => {
      if (this.sending === tracked) this.sending = null;
    });
    this.sending = tracked;
    return tracked;
  }
}

/** bindPageEvents 需要的浏览器对象。测试时可以传假的。 */
export interface PageTargets {
  document: Pick<Document, "addEventListener" | "removeEventListener" | "visibilityState" | "fullscreenElement">;
  window: Pick<Window, "addEventListener" | "removeEventListener">;
  mediaDevices?: Pick<MediaDevices, "addEventListener" | "removeEventListener"> | undefined;
}

/**
 * 监听页面上和面试诚信、网络状况有关的事件，记到 collector 里。
 * 返回一个函数，调用它取消所有监听。
 */
export function bindPageEvents(collector: EventCollector, targets: PageTargets): () => void {
  const { document: doc, window: win, mediaDevices } = targets;
  const unbind: Array<() => void> = [];
  const listen = (
    target: { addEventListener: EventTarget["addEventListener"]; removeEventListener: EventTarget["removeEventListener"] } | undefined,
    type: string,
    handler: (event: Event) => void,
  ) => {
    if (!target) return;
    target.addEventListener(type, handler);
    unbind.push(() => target.removeEventListener(type, handler));
  };

  listen(doc, "visibilitychange", () => {
    const hidden = doc.visibilityState === "hidden";
    collector.record(hidden ? "visibility_hidden" : "visibility_visible");
    // 切走之后页面随时可能被系统回收，趁现在把手里的发出去
    if (hidden) void collector.flush({ keepalive: true });
  });
  listen(win, "blur", () => collector.record("window_blur"));
  listen(win, "focus", () => collector.record("window_focus"));
  listen(doc, "fullscreenchange", () => {
    if (!doc.fullscreenElement) collector.record("fullscreen_exit");
  });
  // 只记发生了，不记内容
  listen(doc, "paste", () => collector.record("paste"));
  listen(doc, "copy", () => collector.record("copy"));
  listen(win, "offline", () => collector.record("network_offline"));
  listen(win, "online", () => {
    collector.record("network_online");
    void collector.flush();
  });
  listen(mediaDevices, "devicechange", () => collector.record("device_changed"));
  listen(win, "pagehide", () => void collector.flush({ keepalive: true }));

  return () => {
    for (const off of unbind) off();
  };
}
