import { describe, expect, it } from "vitest";
import type { FrameEvent, InterviewEvent } from "../api/types";
import { framesByEvent, frameUrl, withFrameEvents } from "./frames";

const process = (id: string, kind: string, at_ms: number | null): InterviewEvent => ({
  id,
  kind,
  at_ms,
  occurred_at: "2026-01-01T00:00:00Z",
  source: "client",
  payload: {},
});

const frame = (id: string, kind: string, at_ms: number | null, frames: number[]): FrameEvent => ({
  id,
  kind,
  at_ms,
  duration_ms: null,
  payload: {},
  frame_at_ms: frames,
});

const events = [
  process("open", "page_opened", null),
  process("start", "interview_started", 0),
  process("hidden", "visibility_hidden", 60_000),
  process("finish", "interview_finished", 120_000),
  process("upload", "upload_finished", null),
];

describe("录像画面检查的结果", () => {
  it("按时间排进过程事件里", () => {
    const merged = withFrameEvents("i-1", events, [
      frame("frame-phone-95000", "phone_visible", 95_000, [95_000, 100_000]),
      frame("frame-face-30000", "face_count_changed", 30_000, [30_000]),
    ]);
    expect(merged.map((e) => e.id)).toEqual([
      "open",
      "start",
      "frame-face-30000",
      "hidden",
      "frame-phone-95000",
      "finish",
      "upload",
    ]);
    expect(merged[4]?.frames).toEqual([
      { at_ms: 95_000, url: "/api/v1/interviews/i-1/frames/95000" },
      { at_ms: 100_000, url: "/api/v1/interviews/i-1/frames/100000" },
    ]);
  });

  it("比结束还晚的排在结束后面，但不排到「结束后」才发生的事件后面", () => {
    const merged = withFrameEvents("i-1", events, [
      frame("late", "face_count_changed", 125_000, [125_000]),
    ]);
    expect(merged.map((e) => e.id).slice(-3)).toEqual(["finish", "late", "upload"]);
  });

  it("没有时间的那条（一共抽了多少帧）放在最后，不带图", () => {
    const merged = withFrameEvents("i-1", events, [frame("checked", "frames_checked", null, [])]);
    expect(merged.at(-1)?.id).toBe("checked");
    expect(merged.at(-1)?.frames).toEqual([]);
  });

  it("没有画面事件时原样返回", () => {
    expect(withFrameEvents("i-1", events, [])).toEqual(events);
  });

  it("过程事件都没有时间时，画面事件排在后面", () => {
    const merged = withFrameEvents(
      "i-1",
      [process("open", "page_opened", null)],
      [frame("f", "phone_visible", 5000, [5000])],
    );
    expect(merged.map((e) => e.id)).toEqual(["open", "f"]);
  });

  it("按证据里的 event_id 找到存了图的帧，没存图的不算", () => {
    const byEvent = framesByEvent("i-1", [
      frame("frame-phone-95000", "phone_visible", 95_000, [95_000]),
      frame("checked", "frames_checked", null, []),
    ]);
    expect(byEvent.get("frame-phone-95000")).toEqual([
      { at_ms: 95_000, url: frameUrl("i-1", 95_000) },
    ]);
    expect(byEvent.has("checked")).toBe(false);
  });
});
