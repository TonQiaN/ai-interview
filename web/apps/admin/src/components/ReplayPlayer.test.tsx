import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Recording } from "../api/types";
import type { TimelineEvent } from "../lib/frames";
import { ReplayPlayer } from "./ReplayPlayer";

afterEach(cleanup);

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
    media_url: `/api/v1/recordings/${n}/media`,
    created_at: "2026-11-16T00:00:00Z",
    ...patch,
  };
}

function show(recordings: Recording[], events: TimelineEvent[] = []) {
  render(
    <ReplayPlayer
      turns={[]}
      events={events}
      recordings={recordings}
      candidateName="林晓"
      elapsedMs={600_000}
    />,
  );
}

describe("回看", () => {
  it("录像能播但不完整时，把原因显示出来", () => {
    show([recording({ error: "候选人那边没有传完这一段录像，结尾可能缺一小段" })]);
    expect(screen.getByText("录像不完整")).toBeTruthy();
    expect(screen.getByText("录像：候选人那边没有传完这一段录像，结尾可能缺一小段。")).toBeTruthy();
  });

  it("有一段没保存下来时，说明原因", () => {
    show([
      recording({}),
      recording({
        segment: 1,
        start_ms: 300_000,
        status: "failed",
        media_url: null,
        error: "候选人那边没有把这一段录像传上来",
      }),
    ]);
    expect(
      screen.getByText(
        "录像第 2 段（从 05:00 开始）：候选人那边没有把这一段录像传上来。这一段只能看逐字稿。",
      ),
    ).toBeTruthy();
  });

  it("一部分能播、另一部分还在整理时，下面说一句；什么都还不能播时只在画面上说", () => {
    show([
      recording({}),
      recording({ segment: 1, start_ms: 300_000, status: "assembling", media_url: null }),
    ]);
    expect(screen.getByText(/录像第 2 段还在上传和整理/)).toBeTruthy();
    cleanup();

    show([recording({ status: "assembling", media_url: null })]);
    expect(screen.getByText("录像还在上传和整理，过一会儿再来看")).toBeTruthy();
    expect(screen.queryByText(/面试结束后要等半小时左右/)).toBeNull();
  });

  it("录像都正常时不多说", () => {
    show([recording({})]);
    expect(screen.queryByText("录像不完整")).toBeNull();
  });

  it("画面降过清晰度时说明不是摄像头的问题；只有录音时不说", () => {
    const reduced: TimelineEvent = {
      id: "0193a000-0000-7000-8000-0000000000e1",
      kind: "interview_started",
      at_ms: 0,
      occurred_at: "2026-11-16T00:00:00Z",
      source: "client",
      payload: { video_kbps: 300, video_reduced: true },
    };
    show([recording({})], [reduced]);
    expect(screen.getByText(/画面比平时糊，不是候选人的摄像头有问题/)).toBeTruthy();
    cleanup();

    show([recording({ track: "microphone", content_type: "audio/webm" })], [reduced]);
    expect(screen.queryByText(/画面比平时糊/)).toBeNull();
    cleanup();

    show([recording({})]);
    expect(screen.queryByText(/画面比平时糊/)).toBeNull();
  });
});
