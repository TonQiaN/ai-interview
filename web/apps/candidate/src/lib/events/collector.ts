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
    this.buffer.push({
      kind,
      at_ms: this.timelineNow(),
      occurred_at: this.wallNow().toISOString(),
      payload,
    });
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
  document: Pick<
    Document,
    "addEventListener" | "removeEventListener" | "visibilityState" | "fullscreenElement"
  > &
    Partial<Pick<Document, "getSelection" | "fullscreenEnabled">>;
  window: Pick<Window, "addEventListener" | "removeEventListener">;
  mediaDevices?:
    | (Pick<MediaDevices, "addEventListener" | "removeEventListener"> &
        Partial<Pick<MediaDevices, "enumerateDevices">>)
    | undefined;
  /** 单调时钟（毫秒），用来算「离开了多久」。默认 performance.now。 */
  now?: () => number;
}

export interface PageEventBinding {
  /** 取消所有监听。 */
  unbind(): void;
  /**
   * 重新记一次当前有哪些设备。拿到摄像头和麦克风权限之后要调一次：
   * 授权前浏览器只肯说「每类有一个」，授权后才给完整的清单，不重记会把这个差别当成换了设备。
   */
  rebaselineDevices(): Promise<void>;
}

/** 每类设备各有几个。只数个数，不带设备名称（名称里可能有人名，比如「某某的 AirPods」）。 */
export interface DeviceCounts {
  camera: number;
  microphone: number;
  speaker: number;
}

const DEVICE_KIND: Record<MediaDeviceKind, keyof DeviceCounts> = {
  videoinput: "camera",
  audioinput: "microphone",
  audiooutput: "speaker",
};

async function countDevices(
  mediaDevices: PageTargets["mediaDevices"],
): Promise<DeviceCounts | null> {
  if (!mediaDevices?.enumerateDevices) return null;
  try {
    const counts: DeviceCounts = { camera: 0, microphone: 0, speaker: 0 };
    for (const device of await mediaDevices.enumerateDevices())
      counts[DEVICE_KIND[device.kind]] += 1;
    return counts;
  } catch {
    return null;
  }
}

/**
 * 这个浏览器里哪几类过程事件采得到。随 page_opened 一起上报，
 * 报告据此区分「没发生」和「采不到」。
 *
 * 失焦（window_blur）没法事先探测：有些应用内置的浏览器就是不触发，所以这里不列，
 * 报告要结合 page_opened 里的 in_app_browser 来判断。
 */
export function collectableSignals(
  targets: PageTargets,
  navigatorLike: { onLine?: boolean } = {},
): Record<string, boolean> {
  return {
    visibility: typeof targets.document.visibilityState === "string",
    fullscreen: targets.document.fullscreenEnabled === true,
    device_change:
      Boolean(targets.mediaDevices?.enumerateDevices) &&
      "ondevicechange" in (targets.mediaDevices ?? {}),
    network: typeof navigatorLike.onLine === "boolean",
    clipboard: true,
  };
}

/**
 * 监听页面上和面试诚信、网络状况有关的事件，记到 collector 里。
 *
 * 成对的事件（切走和回来、失焦和聚焦、断网和恢复），回来的那一条带 duration_ms，表示离开了多久。
 * 粘贴和复制只记发生了、有多少个字符，不记内容。
 */
export function bindPageEvents(collector: EventCollector, targets: PageTargets): PageEventBinding {
  const { document: doc, window: win, mediaDevices } = targets;
  const now = targets.now ?? (() => performance.now());
  const unbind: Array<() => void> = [];
  const listen = (
    target:
      | {
          addEventListener: EventTarget["addEventListener"];
          removeEventListener: EventTarget["removeEventListener"];
        }
      | undefined,
    type: string,
    handler: (event: Event) => void,
  ) => {
    if (!target) return;
    target.addEventListener(type, handler);
    unbind.push(() => target.removeEventListener(type, handler));
  };

  /** 每一对事件里「离开」那一头发生的时刻。 */
  const leftAt = new Map<string, number>();
  const leave = (pair: string, kind: string) => {
    leftAt.set(pair, now());
    collector.record(kind);
  };
  const comeBack = (pair: string, kind: string) => {
    const since = leftAt.get(pair);
    leftAt.delete(pair);
    collector.record(
      kind,
      since === undefined ? {} : { duration_ms: Math.max(0, Math.round(now() - since)) },
    );
  };

  listen(doc, "visibilitychange", () => {
    if (doc.visibilityState === "hidden") {
      leave("visibility", "visibility_hidden");
      // 切走之后页面随时可能被系统回收，趁现在把手里的发出去
      void collector.flush({ keepalive: true });
    } else {
      comeBack("visibility", "visibility_visible");
    }
  });
  listen(win, "blur", () => leave("focus", "window_blur"));
  listen(win, "focus", () => comeBack("focus", "window_focus"));
  listen(doc, "fullscreenchange", () => {
    if (!doc.fullscreenElement) collector.record("fullscreen_exit");
  });
  listen(doc, "paste", (event) => {
    // 只数字符数，内容不留、不上报
    const chars = (event as ClipboardEvent).clipboardData?.getData("text").length;
    collector.record("paste", chars === undefined ? {} : { chars });
  });
  listen(doc, "copy", () => {
    const chars = doc.getSelection?.()?.toString().length;
    collector.record("copy", chars === undefined ? {} : { chars });
  });
  listen(win, "offline", () => leave("network", "network_offline"));
  listen(win, "online", () => {
    comeBack("network", "network_online");
    void collector.flush();
  });

  let devices: DeviceCounts | null = null;
  const rebaselineDevices = async () => {
    devices = await countDevices(mediaDevices);
  };
  void rebaselineDevices();
  listen(mediaDevices, "devicechange", () => {
    const before = devices;
    void countDevices(mediaDevices).then((after) => {
      devices = after;
      if (!before || !after) return collector.record("device_changed");
      const kinds = (Object.keys(after) as Array<keyof DeviceCounts>).filter(
        (kind) => after[kind] !== before[kind],
      );
      collector.record("device_changed", { kinds, before, after });
    });
  });
  listen(win, "pagehide", () => void collector.flush({ keepalive: true }));

  return {
    unbind: () => {
      for (const off of unbind) off();
    },
    rebaselineDevices,
  };
}
