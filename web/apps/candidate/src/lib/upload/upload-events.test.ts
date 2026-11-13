import { describe, expect, it } from "vitest";
import { UploadEventTracker } from "./upload-events";
import type { UploadStatus } from "./uploader";

function setup(maxStalls?: number) {
  const events: Array<{ kind: string; payload: Record<string, unknown> }> = [];
  let clock = 0;
  const tracker = new UploadEventTracker({
    record: (kind, payload) => events.push({ kind, payload }),
    now: () => clock,
    maxStalls,
  });
  return {
    events,
    tracker,
    advance: (ms: number) => {
      clock += ms;
    },
  };
}

function status(patch: Partial<UploadStatus>): UploadStatus {
  return {
    state: "uploading",
    pendingChunks: 0,
    pendingBytes: 0,
    uploadedBytes: 0,
    consecutiveFailures: 0,
    lastError: null,
    ...patch,
  };
}

describe("UploadEventTracker", () => {
  it("一直顺利时什么都不记", () => {
    const { events, tracker } = setup();
    tracker.observe(status({ state: "uploading", pendingChunks: 2 }));
    tracker.observe(status({ state: "uploading", uploadedBytes: 100, pendingChunks: 1 }));
    tracker.observe(status({ state: "idle", uploadedBytes: 200 }));
    expect(events).toEqual([]);
  });

  it("重试多少次只记一次停顿，传上去一片才算恢复", () => {
    const { events, tracker, advance } = setup();
    tracker.observe(status({ state: "retrying", consecutiveFailures: 1, pendingChunks: 3 }));
    advance(1_000);
    // 重试之前状态先变回「上传中」，然后又失败
    tracker.observe(status({ state: "uploading", consecutiveFailures: 1, pendingChunks: 3 }));
    tracker.observe(status({ state: "retrying", consecutiveFailures: 2, pendingChunks: 4 }));
    advance(2_000);
    tracker.observe(status({ state: "uploading", consecutiveFailures: 2, pendingChunks: 4 }));
    expect(events).toEqual([
      { kind: "upload_stalled", payload: { cause: "retrying", pending_chunks: 3 } },
    ]);

    advance(500);
    tracker.observe(status({ state: "uploading", uploadedBytes: 50, pendingChunks: 3 }));
    expect(events[1]).toEqual({
      kind: "upload_recovered",
      payload: { stalled_ms: 3_500, failures: 2, pending_chunks: 3 },
    });
    // 恢复之后继续传，不再记
    tracker.observe(status({ state: "uploading", uploadedBytes: 100, pendingChunks: 2 }));
    expect(events).toHaveLength(2);
  });

  it("断网后恢复，队列传空也算恢复", () => {
    const { events, tracker, advance } = setup();
    tracker.observe(status({ state: "offline", pendingChunks: 1, uploadedBytes: 10 }));
    advance(20_000);
    tracker.observe(status({ state: "uploading", pendingChunks: 1, uploadedBytes: 10 }));
    expect(events).toHaveLength(1);
    tracker.observe(status({ state: "idle", uploadedBytes: 10 }));
    expect(events.map((event) => event.kind)).toEqual(["upload_stalled", "upload_recovered"]);
    expect(events[0]?.payload).toEqual({ cause: "offline", pending_chunks: 1 });
    expect(events[1]?.payload).toMatchObject({ stalled_ms: 20_000, failures: 0 });
  });

  it("重试之后彻底失败，再记一条", () => {
    const { events, tracker } = setup();
    tracker.observe(status({ state: "retrying", consecutiveFailures: 1, pendingChunks: 2 }));
    tracker.observe(status({ state: "failed", consecutiveFailures: 1, pendingChunks: 2 }));
    tracker.observe(status({ state: "failed", consecutiveFailures: 1, pendingChunks: 2 }));
    expect(events.map((event) => event.payload.cause)).toEqual(["retrying", "failed"]);
  });

  it("一上来就失败只记一条", () => {
    const { events, tracker } = setup();
    tracker.observe(status({ state: "failed", pendingChunks: 2 }));
    expect(events).toEqual([
      { kind: "upload_stalled", payload: { cause: "failed", pending_chunks: 2 } },
    ]);
  });

  it("网络一直抖时，停顿记到上限为止", () => {
    const { events, tracker } = setup(2);
    for (let round = 1; round <= 4; round += 1) {
      tracker.observe(status({ state: "retrying", consecutiveFailures: 1 }));
      tracker.observe(status({ state: "uploading", uploadedBytes: round * 10 }));
    }
    expect(events.map((event) => event.kind)).toEqual([
      "upload_stalled",
      "upload_recovered",
      "upload_stalled",
      "upload_recovered",
    ]);
  });
});
