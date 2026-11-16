import { describe, expect, it } from "vitest";
import { MemoryChunkStore, type SegmentMeta } from "./chunk-store";
import { discardSession, discardStale, STALE_AFTER_MS } from "./leftovers";

const NOW = 1_800_000_000_000;
const DAY = 24 * 60 * 60 * 1000;

async function addSegment(
  store: MemoryChunkStore,
  sessionKey: string,
  segmentId: string,
  createdAt: number | undefined,
  chunks = 2,
): Promise<void> {
  const meta: SegmentMeta = {
    sessionKey,
    track: "camera",
    segmentId,
    mimeType: "video/webm",
    startMs: 0,
    totalChunks: null,
  };
  await store.putSegment(createdAt === undefined ? meta : { ...meta, createdAt });
  for (let seq = 0; seq < chunks; seq += 1) {
    await store.putChunk({ sessionKey, segmentId, seq, bytes: new ArrayBuffer(4), sha256: "x" });
  }
}

const left = async (store: MemoryChunkStore): Promise<string[]> =>
  (await store.listAllSegments()).map((item) => `${item.sessionKey}/${item.segmentId}`).sort();

describe("链接失效后清掉这场面试留在本机的录像", () => {
  it("这场面试的段和分片都删掉，别的面试的不动", async () => {
    const store = new MemoryChunkStore();
    await addSegment(store, "mine", "s1", NOW);
    await addSegment(store, "mine", "s2", NOW);
    await addSegment(store, "other", "s1", NOW);

    expect(await discardSession(store, "mine")).toBe(2);

    expect(await left(store)).toEqual(["other/s1"]);
    expect(await store.listChunks("mine", "s1")).toEqual([]);
    expect(await store.listChunks("mine", "s2")).toEqual([]);
    expect(await store.listChunks("other", "s1")).toHaveLength(2);
  });

  it("本机什么都没留的时候不报错", async () => {
    expect(await discardSession(new MemoryChunkStore(), "mine")).toBe(0);
  });
});

describe("清掉别的面试留下的旧录像", () => {
  it("超过 7 天的删掉，没到的留着接着传", async () => {
    const store = new MemoryChunkStore();
    await addSegment(store, "old", "s1", NOW - STALE_AFTER_MS);
    await addSegment(store, "older", "s1", NOW - 30 * DAY);
    await addSegment(store, "recent", "s1", NOW - STALE_AFTER_MS + 1);
    await addSegment(store, "yesterday", "s1", NOW - DAY);

    expect(await discardStale(store, { keep: "mine", now: NOW })).toBe(2);

    expect(await left(store)).toEqual(["recent/s1", "yesterday/s1"]);
    expect(await store.listChunks("old", "s1")).toEqual([]);
    expect(await store.listChunks("recent", "s1")).toHaveLength(2);
  });

  it("当前这场面试的录像再旧也不动：能不能传由服务端说了算", async () => {
    const store = new MemoryChunkStore();
    await addSegment(store, "mine", "s1", NOW - 30 * DAY);

    expect(await discardStale(store, { keep: "mine", now: NOW })).toBe(0);
    expect(await left(store)).toEqual(["mine/s1"]);
  });

  it("没记下录制时间的按很久以前算", async () => {
    const store = new MemoryChunkStore();
    await addSegment(store, "legacy", "s1", undefined);

    expect(await discardStale(store, { keep: "mine", now: NOW })).toBe(1);
    expect(await left(store)).toEqual([]);
  });

  it("设备的钟被调回去了（录制时间在未来）：不删", async () => {
    const store = new MemoryChunkStore();
    await addSegment(store, "future", "s1", NOW + DAY);

    expect(await discardStale(store, { keep: "mine", now: NOW })).toBe(0);
  });
});
