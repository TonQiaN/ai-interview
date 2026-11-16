import { describe, expect, it } from "vitest";
import { createMockServer } from "../api/mock/server";
import type { Recording } from "../api/types";
import { recordingNotes, reducedVideoNote } from "./recordings";

let n = 0;
function recording(patch: Partial<Recording>): Recording {
  n += 1;
  return {
    id: `0193a000-0000-7000-8000-${String(n).padStart(12, "0")}`,
    track: "camera",
    segment: 0,
    content_type: "video/webm",
    status: "ready",
    size_bytes: 1000,
    start_ms: 0,
    duration_ms: 60_000,
    error: null,
    media_url: "/api/v1/recordings/x/media",
    created_at: "2026-11-16T00:00:00Z",
    ...patch,
  };
}

describe("录制情况的说明", () => {
  it("都正常时什么也不说", () => {
    expect(recordingNotes([recording({}), recording({ track: "microphone" })])).toEqual([]);
  });

  it("能播但不完整的，把后端写的原因原样说出来；录像和录音同样的问题并成一句", () => {
    const error = "候选人那边没有传完这一段录像，结尾可能缺一小段";
    const notes = recordingNotes([recording({ error }), recording({ track: "microphone", error })]);
    expect(notes).toEqual([
      expect.objectContaining({
        tone: "warning",
        text: "录像和录音：候选人那边没有传完这一段录像，结尾可能缺一小段。",
      }),
    ]);
  });

  it("有好几段时说清楚是第几段、从哪里开始", () => {
    const notes = recordingNotes([
      recording({}),
      recording({
        segment: 1,
        start_ms: 754_000,
        error: "候选人那边没有传完这一段录像，中间缺了一部分，只有缺口之前的内容",
      }),
    ]);
    expect(notes.map((x) => x.text)).toEqual([
      "录像第 2 段（从 12:34 开始）：候选人那边没有传完这一段录像，中间缺了一部分，只有缺口之前的内容。",
    ]);
  });

  it("没保存下来的段说明原因，并说这一段只能看逐字稿；没写原因也有一句话", () => {
    const notes = recordingNotes([
      recording({
        status: "failed",
        media_url: null,
        error: "候选人那边没有把这一段录像传上来",
      }),
      recording({ track: "microphone", status: "failed", media_url: null }),
    ]);
    expect(notes.map((x) => x.text)).toEqual([
      "录像：候选人那边没有把这一段录像传上来。这一段只能看逐字稿。",
      "录音：这一段没有保存下来。这一段只能看逐字稿。",
    ]);
  });

  it("演示数据里断过网的那场面试，第一段说明不完整", () => {
    const server = createMockServer({ latencyMs: 0 });
    const cut = [...server.db.recordings.values()].find((items) => items.some((r) => r.error));
    if (!cut) throw new Error("演示数据里没有不完整的录像");
    expect(recordingNotes(cut).map((x) => x.text)).toEqual([
      "录像第 1 段和录音第 1 段（从 00:00 开始）：候选人那边没有传完这一段录像，结尾可能缺一小段。",
    ]);
  });

  it("还在上传和整理的单独说一句，不算有问题", () => {
    const notes = recordingNotes([
      recording({ status: "assembling", media_url: null }),
      recording({ track: "microphone", status: "uploading", media_url: null }),
    ]);
    expect(notes).toHaveLength(1);
    expect(notes[0]?.tone).toBe("info");
    expect(notes[0]?.text).toMatch(/^录像、录音还在上传和整理/);
  });
});

describe("画面降过清晰度的说明", () => {
  const started = (at_ms: number | null, payload: Record<string, unknown>) => ({
    kind: "interview_started",
    at_ms,
    payload,
  });

  it("没降过就不说", () => {
    expect(reducedVideoNote([])).toBeNull();
    expect(reducedVideoNote([started(0, { video_kbps: 900, video_reduced: false })])).toBeNull();
    expect(reducedVideoNote([{ kind: "tab_hidden", at_ms: 5000, payload: {} }])).toBeNull();
  });

  it("整场都降了，说这场面试", () => {
    expect(reducedVideoNote([started(0, { video_kbps: 300, video_reduced: true })])).toBe(
      "候选人的上行网速慢，面试页把这场面试录像的画面降了清晰度（声音没有降）。画面比平时糊，不是候选人的摄像头有问题。",
    );
  });

  it("只有接着面的那一段降了，说从什么时候开始", () => {
    expect(
      reducedVideoNote([
        started(0, { video_kbps: 900, video_reduced: false }),
        started(185_000, { resumed: true, video_kbps: 300, video_reduced: true }),
      ]),
    ).toBe(
      "候选人的上行网速慢，面试页把从 03:05 开始的那一段录像的画面降了清晰度（声音没有降）。画面比平时糊，不是候选人的摄像头有问题。",
    );
  });
});
