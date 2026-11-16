import { describe, expect, it } from "vitest";
import { fitRecordingBitrate, MIN_VIDEO_BITS_PER_SECOND } from "./bitrate";

const configured = { videoBitsPerSecond: 500_000, audioBitsPerSecond: 48_000 };

describe("按上行网速定录像的码率", () => {
  it("上行留出五成余量还装得下：按服务端给的录", () => {
    const full = { ...configured, reduced: false, fits: true };
    expect(fitRecordingBitrate(configured, 2_000)).toEqual(full);
    // 548 kbps 的录像，正好 1.5 倍
    expect(fitRecordingBitrate(configured, 822)).toEqual(full);
  });

  it("装不下：声音不动，画面降到装得下", () => {
    // 600 kbps 留出五成余量是 400，减去声音的 48 剩 352
    expect(fitRecordingBitrate(configured, 600)).toEqual({
      videoBitsPerSecond: 352_000,
      audioBitsPerSecond: 48_000,
      reduced: true,
      fits: true,
    });
    expect(fitRecordingBitrate(configured, 821).reduced).toBe(true);
  });

  it("再慢也不低于最低的画面码率，再低就看不清人了；这时候上行还是装不下", () => {
    expect(fitRecordingBitrate(configured, 200)).toEqual({
      videoBitsPerSecond: MIN_VIDEO_BITS_PER_SECOND,
      audioBitsPerSecond: 48_000,
      reduced: true,
      fits: false,
    });
    expect(fitRecordingBitrate(configured, 1).videoBitsPerSecond).toBe(MIN_VIDEO_BITS_PER_SECOND);
    // 最低的画面码率加声音是 168 kbps，1.5 倍是 252
    expect(fitRecordingBitrate(configured, 252).fits).toBe(true);
    expect(fitRecordingBitrate(configured, 251).fits).toBe(false);
  });

  it("没量出上行：按服务端给的录", () => {
    expect(fitRecordingBitrate(configured, null)).toEqual({
      ...configured,
      reduced: false,
      fits: true,
    });
  });

  it("服务端给的画面码率本来就比最低的还低：不往上调", () => {
    const low = { videoBitsPerSecond: 80_000, audioBitsPerSecond: 32_000 };
    expect(fitRecordingBitrate(low, 100)).toEqual({ ...low, reduced: false, fits: false });
  });

  it("只录声音时要求低得多", () => {
    const audioOnly = { videoBitsPerSecond: 0, audioBitsPerSecond: 48_000 };
    expect(fitRecordingBitrate(audioOnly, 100)).toEqual({
      ...audioOnly,
      reduced: false,
      fits: true,
    });
    expect(fitRecordingBitrate(audioOnly, 60).fits).toBe(false);
  });
});
