import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientEvent } from "../api/room-types";
import { bindPageEvents, collectableSignals, EventCollector } from "./collector";

function setup(options: { maxBatch?: number; maxBuffered?: number } = {}) {
  const sent: Array<{ events: ClientEvent[]; keepalive: boolean }> = [];
  const control = { fail: false, hang: false };
  let timeline: number | null = 1_000;
  const collector = new EventCollector({
    send: async (events, { keepalive }) => {
      if (control.hang) await new Promise(() => {});
      if (control.fail) throw new Error("断网");
      sent.push({ events, keepalive });
    },
    timelineNow: () => timeline,
    wallNow: () => new Date(Date.now()),
    ...options,
  });
  return {
    collector,
    sent,
    control,
    setTimeline: (value: number | null) => {
      timeline = value;
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-11-13T02:00:00Z"));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("过程事件收集", () => {
  it("记下事件的时间，定时成批发送", async () => {
    const { collector, sent, setTimeline } = setup();
    collector.start();
    setTimeline(null);
    collector.record("device_check", { camera: "ok" });
    setTimeline(12_345);
    collector.record("visibility_hidden");
    expect(sent).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(5_000);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.events).toEqual([
      { kind: "device_check", at_ms: null, occurred_at: "2026-11-13T02:00:00.000Z", payload: { camera: "ok" } },
      { kind: "visibility_hidden", at_ms: 12_345, occurred_at: "2026-11-13T02:00:00.000Z", payload: {} },
    ]);
    expect(collector.pending).toBe(0);

    // 没有新事件就不发空请求
    await vi.advanceTimersByTimeAsync(10_000);
    expect(sent).toHaveLength(1);
    collector.stop();
  });

  it("攒够一批立刻发", async () => {
    const { collector, sent } = setup({ maxBatch: 3 });
    collector.record("a");
    collector.record("b");
    expect(sent).toHaveLength(0);
    collector.record("c");
    await vi.advanceTimersByTimeAsync(0);
    expect(sent[0]?.events.map((event) => event.kind)).toEqual(["a", "b", "c"]);
  });

  it("发送失败时事件留着，下次按原来的顺序重发", async () => {
    const { collector, sent, control } = setup();
    control.fail = true;
    collector.record("a");
    collector.record("b");
    await collector.flush();
    expect(sent).toHaveLength(0);
    expect(collector.pending).toBe(2);

    collector.record("c");
    control.fail = false;
    await collector.flush();
    expect(sent[0]?.events.map((event) => event.kind)).toEqual(["a", "b", "c"]);
  });

  it("攒得太多时丢最旧的，并报告丢了几条", async () => {
    const { collector, sent, control } = setup({ maxBatch: 100, maxBuffered: 5 });
    control.fail = true;
    for (let i = 0; i < 8; i += 1) collector.record(`e${i}`);
    expect(collector.pending).toBe(5);

    control.fail = false;
    await collector.flush();
    expect(sent[0]?.events.map((event) => event.kind)).toEqual(["e3", "e4", "e5", "e6", "e7", "events_dropped"]);
    expect(sent[0]?.events.at(-1)?.payload).toEqual({ count: 3 });
  });

  it("页面要关时不等前一批回来，直接带 keepalive 发", async () => {
    const { collector, sent, control } = setup();
    control.hang = true;
    collector.record("a");
    void collector.flush(); // 这一批卡在路上
    control.hang = false;
    collector.record("b");
    await collector.flush({ keepalive: true });
    expect(sent).toEqual([{ events: [expect.objectContaining({ kind: "b" })], keepalive: true }]);
  });
});

describe("页面事件监听", () => {
  class FakeDocument extends EventTarget {
    visibilityState: DocumentVisibilityState = "visible";
    fullscreenElement: Element | null = null;
    fullscreenEnabled = true;
    selection = "";
    getSelection = () => ({ toString: () => this.selection }) as Selection;
  }

  class FakeMediaDevices extends EventTarget {
    ondevicechange = null;
    devices: MediaDeviceKind[] = ["videoinput", "audioinput", "audiooutput"];
    enumerateDevices = async () => this.devices.map((kind) => ({ kind, label: "不该被上报的设备名" }) as MediaDeviceInfo);
  }

  function bind() {
    const context = setup();
    const doc = new FakeDocument();
    const win = new EventTarget();
    const mediaDevices = new FakeMediaDevices();
    const targets = {
      document: doc as unknown as Document,
      window: win as unknown as Window,
      mediaDevices: mediaDevices as unknown as MediaDevices,
      now: () => Date.now(),
    };
    const binding = bindPageEvents(context.collector, targets);
    return { ...context, doc, win, mediaDevices, targets, binding };
  }

  function paste(text: string): Event {
    const event = new Event("paste");
    Object.defineProperty(event, "clipboardData", { value: { getData: () => text } });
    return event;
  }

  it("把切页面、失焦、粘贴、断网、换设备都记下来", async () => {
    const { collector, sent, doc, win, mediaDevices } = bind();
    await vi.advanceTimersByTimeAsync(0);

    win.dispatchEvent(new Event("blur"));
    doc.visibilityState = "hidden";
    doc.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(0);
    // 切走时立刻发一次，免得页面被回收后丢掉
    expect(sent[0]).toMatchObject({ keepalive: true });
    expect(sent[0]?.events.map((event) => event.kind)).toEqual(["window_blur", "visibility_hidden"]);

    doc.visibilityState = "visible";
    doc.dispatchEvent(new Event("visibilitychange"));
    win.dispatchEvent(new Event("focus"));
    doc.dispatchEvent(paste("粘贴进来的十个字符哦"));
    doc.selection = "选中的字";
    doc.dispatchEvent(new Event("copy"));
    win.dispatchEvent(new Event("offline"));
    doc.fullscreenElement = {} as Element;
    doc.dispatchEvent(new Event("fullscreenchange")); // 进入全屏不记
    doc.fullscreenElement = null;
    doc.dispatchEvent(new Event("fullscreenchange"));
    win.dispatchEvent(new Event("online"));
    await vi.advanceTimersByTimeAsync(0);

    expect(sent[1]?.events.map((event) => event.kind)).toEqual([
      "visibility_visible",
      "window_focus",
      "paste",
      "copy",
      "network_offline",
      "fullscreen_exit",
      "network_online",
    ]);
    // 粘贴和复制只记发生了和字符数，不带内容
    const clipboard = sent[1]?.events.filter((event) => event.kind === "paste" || event.kind === "copy");
    expect(clipboard?.map((event) => event.payload)).toEqual([{ chars: 10 }, { chars: 4 }]);
    expect(JSON.stringify(sent)).not.toContain("粘贴进来");
    expect(collector.pending).toBe(0);
    void mediaDevices;
  });

  it("成对的事件，回来的那一条带上离开了多久", async () => {
    const { collector, sent, doc, win } = bind();
    await vi.advanceTimersByTimeAsync(0);

    win.dispatchEvent(new Event("focus")); // 没有对应的失焦：不带时长
    win.dispatchEvent(new Event("blur"));
    await vi.advanceTimersByTimeAsync(1200);
    win.dispatchEvent(new Event("focus"));
    win.dispatchEvent(new Event("offline"));
    await vi.advanceTimersByTimeAsync(700);
    win.dispatchEvent(new Event("online"));
    doc.visibilityState = "hidden";
    doc.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(18_000);
    doc.visibilityState = "visible";
    doc.dispatchEvent(new Event("visibilitychange"));
    await collector.flush();

    const events = sent.flatMap((batch) => batch.events);
    expect(events.filter((event) => event.kind === "window_focus").map((event) => event.payload)).toEqual([{}, { duration_ms: 1200 }]);
    expect(events.find((event) => event.kind === "network_online")?.payload).toEqual({ duration_ms: 700 });
    expect(events.find((event) => event.kind === "visibility_visible")?.payload).toEqual({ duration_ms: 18_000 });
  });

  it("换设备时报是哪一类变了，只数个数，不带设备名", async () => {
    const { collector, sent, mediaDevices, binding } = bind();
    await vi.advanceTimersByTimeAsync(0);

    // 授权之后浏览器给出完整清单，这不算换设备
    mediaDevices.devices = ["videoinput", "audioinput", "audioinput", "audiooutput"];
    await binding.rebaselineDevices();

    mediaDevices.devices = ["videoinput", "audioinput", "audiooutput", "audiooutput"];
    mediaDevices.dispatchEvent(new Event("devicechange"));
    await vi.advanceTimersByTimeAsync(0);
    await collector.flush();

    const changed = sent.flatMap((batch) => batch.events).filter((event) => event.kind === "device_changed");
    expect(changed.map((event) => event.payload)).toEqual([
      {
        kinds: ["microphone", "speaker"],
        before: { camera: 1, microphone: 2, speaker: 1 },
        after: { camera: 1, microphone: 1, speaker: 2 },
      },
    ]);
    expect(JSON.stringify(changed)).not.toContain("设备名");
  });

  it("报告这个浏览器里哪几类事件采得到", () => {
    const { targets, doc } = bind();
    expect(collectableSignals(targets, { onLine: true })).toEqual({
      visibility: true,
      fullscreen: true,
      device_change: true,
      network: true,
      clipboard: true,
    });
    doc.fullscreenEnabled = false;
    expect(collectableSignals({ ...targets, mediaDevices: undefined }, {})).toMatchObject({
      fullscreen: false,
      device_change: false,
      network: false,
    });
  });

  it("关页面时把剩下的发出去；解绑后不再记录", async () => {
    const { collector, sent, doc, win, binding } = bind();
    doc.dispatchEvent(new Event("paste"));
    win.dispatchEvent(new Event("pagehide"));
    await vi.advanceTimersByTimeAsync(0);
    expect(sent[0]).toMatchObject({ keepalive: true });

    binding.unbind();
    doc.dispatchEvent(new Event("paste"));
    win.dispatchEvent(new Event("blur"));
    expect(collector.pending).toBe(0);
  });
});
