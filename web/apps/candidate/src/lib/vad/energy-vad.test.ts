import { describe, expect, it } from "vitest";
import { EnergyVad, rmsToDb, type VadEvent } from "./energy-vad";

const FRAME_MS = 20;

/** dBFS 转回线性 RMS，方便用分贝写测试。 */
function db(value: number): number {
  return 10 ** (value / 20);
}

/** 以固定音量送入一段时间，返回期间产生的事件。 */
function feed(vad: EnergyVad, levelDb: number, durationMs: number): VadEvent[] {
  const events: VadEvent[] = [];
  for (let t = 0; t < durationMs; t += FRAME_MS) {
    const event = vad.process(db(levelDb), FRAME_MS);
    if (event) events.push(event);
  }
  return events;
}

describe("rmsToDb", () => {
  it("满幅是 0 dBFS，十分之一是 -20 dBFS", () => {
    expect(rmsToDb(1)).toBeCloseTo(0);
    expect(rmsToDb(0.1)).toBeCloseTo(-20);
  });

  it("静音和非法值不会得到负无穷或 NaN", () => {
    expect(rmsToDb(0)).toBe(-100);
    expect(rmsToDb(Number.NaN)).toBe(-100);
    expect(rmsToDb(-1)).toBe(-100);
  });
});

describe("EnergyVad", () => {
  it("安静房间里开口说话：先 speech_start，停下后 speech_end", () => {
    const vad = new EnergyVad();
    expect(feed(vad, -65, 1000)).toEqual([]);
    expect(feed(vad, -25, 1000)).toEqual([{ type: "speech_start" }]);
    expect(vad.snapshot().speaking).toBe(true);
    expect(feed(vad, -65, 1000)).toEqual([{ type: "speech_end", quietMs: 400 }]);
    expect(vad.snapshot().speaking).toBe(false);
  });

  it("很短的响声（咳嗽、敲桌子）不算开始说话", () => {
    const vad = new EnergyVad();
    feed(vad, -65, 1000);
    expect(feed(vad, -20, 100)).toEqual([]);
    expect(feed(vad, -65, 500)).toEqual([]);
    expect(vad.snapshot().speaking).toBe(false);
  });

  it("说话中间的短停顿不算停止说话", () => {
    const vad = new EnergyVad();
    feed(vad, -65, 500);
    feed(vad, -25, 600);
    expect(feed(vad, -65, 300)).toEqual([]);
    expect(feed(vad, -25, 600)).toEqual([]);
    expect(vad.snapshot().speaking).toBe(true);
  });

  it("一直有背景噪声时，噪声本身不算说话", () => {
    const vad = new EnergyVad();
    // 风扇声 -42 dBFS，从一开始就有
    expect(feed(vad, -42, 20_000)).toEqual([]);
    expect(vad.snapshot().floorDb).toBeCloseTo(-42, 0);
    // 在噪声之上说话仍然能检测到
    expect(feed(vad, -22, 500)).toEqual([{ type: "speech_start" }]);
    // 回到噪声水平算停止
    expect(feed(vad, -42, 1000)).toEqual([{ type: "speech_end", quietMs: 400 }]);
  });

  it("安静中途突然出现持续噪声：几秒内适应，之后不再当成说话", () => {
    const vad = new EnergyVad();
    feed(vad, -65, 2000);
    // 空调启动
    const during = feed(vad, -42, 5000);
    expect(during.at(-1)).toEqual({ type: "speech_end", quietMs: 400 });
    expect(vad.snapshot().speaking).toBe(false);
    expect(feed(vad, -42, 20_000)).toEqual([]);
    // 在噪声之上说话仍然能检测到
    expect(feed(vad, -22, 500)).toEqual([{ type: "speech_start" }]);
  });

  it("长时间说话不会把底噪抬到说话的音量", () => {
    const vad = new EnergyVad();
    feed(vad, -60, 1000);
    // 一分钟连续说话：每个字 180 毫秒，字间 60 毫秒回落到接近底噪
    const events: VadEvent[] = [];
    for (let t = 0; t < 60_000; t += 240) {
      events.push(...feed(vad, -25, 180), ...feed(vad, -58, 60));
    }
    expect(events).toEqual([{ type: "speech_start" }]);
    expect(vad.snapshot().floorDb).toBeLessThan(-55);
    expect(feed(vad, -60, 1000)).toEqual([{ type: "speech_end", quietMs: 400 }]);
  });

  it("极安静的环境里，很轻的声音不会被当成说话", () => {
    const vad = new EnergyVad();
    feed(vad, -95, 2000);
    // 比底噪高 30 dB，但绝对音量仍然很低
    expect(feed(vad, -65, 1000)).toEqual([]);
  });

  it("reset 之后要重新满足持续时间才会再次触发", () => {
    const vad = new EnergyVad();
    feed(vad, -65, 500);
    feed(vad, -25, 500);
    expect(vad.snapshot().speaking).toBe(true);
    vad.reset();
    expect(vad.snapshot().speaking).toBe(false);
    expect(feed(vad, -25, 100)).toEqual([]);
    expect(feed(vad, -25, 200)).toEqual([{ type: "speech_start" }]);
  });
});
