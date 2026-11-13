import { describe, expect, it } from "vitest";
import { MemoryChunkStore, type SegmentMeta } from "./chunk-store";
import { closeOrphanSegments } from "./orphans";

const sessionKey = "s";

function segment(segmentId: string, totalChunks: number | null = null): SegmentMeta {
  return {
    sessionKey,
    track: "camera",
    segmentId,
    mimeType: "video/webm",
    startMs: 0,
    totalChunks,
  };
}

async function addChunk(store: MemoryChunkStore, segmentId: string, seq: number) {
  await store.putChunk({ sessionKey, segmentId, seq, bytes: new ArrayBuffer(4), sha256: "x" });
}

const never = { receivedChunks: async () => Promise.reject(new Error("不该问服务端")) };

describe("收尾上次没结束的录制段", () => {
  it("本地还有分片：总数是最大序号加一，不用问服务端", async () => {
    const store = new MemoryChunkStore();
    await store.putSegment(segment("a"));
    await addChunk(store, "a", 7);
    await addChunk(store, "a", 8);

    expect(await closeOrphanSegments(store, sessionKey, never)).toBe(0);
    expect((await store.listSegments(sessionKey))[0]?.totalChunks).toBe(9);
  });

  it("本地已经传空：按服务端收到的序号算", async () => {
    const store = new MemoryChunkStore();
    await store.putSegment(segment("a"));
    const api = { receivedChunks: async () => [0, 1, 2, 3] };
    expect(await closeOrphanSegments(store, sessionKey, api)).toBe(0);
    expect((await store.listSegments(sessionKey))[0]?.totalChunks).toBe(4);
  });

  it("两边都没有分片的段直接删掉", async () => {
    const store = new MemoryChunkStore();
    await store.putSegment(segment("a"));
    await closeOrphanSegments(store, sessionKey, { receivedChunks: async () => [] });
    expect(await store.listSegments(sessionKey)).toEqual([]);
  });

  it("问不到服务端时留到下次", async () => {
    const store = new MemoryChunkStore();
    await store.putSegment(segment("a"));
    expect(await closeOrphanSegments(store, sessionKey, never)).toBe(1);
    expect((await store.listSegments(sessionKey))[0]?.totalChunks).toBeNull();
  });

  it("已经正常结束的段不动", async () => {
    const store = new MemoryChunkStore();
    await store.putSegment(segment("a", 5));
    await closeOrphanSegments(store, sessionKey, never);
    expect((await store.listSegments(sessionKey))[0]?.totalChunks).toBe(5);
  });
});
