// auto 把 IDBKeyRange 等全局对象装好；每个用例再用独立的 IDBFactory，互不影响
import "fake-indexeddb/auto";
import { IDBFactory } from "fake-indexeddb";
import { describe, expect, it } from "vitest";
import { type ChunkStore, MemoryChunkStore, type SegmentMeta, sha256Hex } from "./chunk-store";
import { IndexedDbChunkStore } from "./idb-chunk-store";

function bytes(text: string): ArrayBuffer {
  return new TextEncoder().encode(text).buffer as ArrayBuffer;
}

function segment(sessionKey: string, segmentId: string, startMs: number): SegmentMeta {
  return {
    sessionKey,
    track: "camera",
    segmentId,
    mimeType: "video/webm",
    startMs,
    totalChunks: null,
  };
}

describe("sha256Hex", () => {
  it("和标准测试向量一致", async () => {
    expect(await sha256Hex(bytes("abc"))).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });
});

// 两个实现要满足同一套行为
const implementations: Array<[string, () => Promise<ChunkStore>]> = [
  ["MemoryChunkStore", async () => new MemoryChunkStore()],
  ["IndexedDbChunkStore", async () => IndexedDbChunkStore.open(new IDBFactory())],
];

describe.each(implementations)("%s", (_name, create) => {
  it("段按开始时间排序，只列出指定面试的", async () => {
    const store = await create();
    await store.putSegment(segment("a", "late", 3000));
    await store.putSegment(segment("a", "early", 1000));
    await store.putSegment(segment("b", "other", 2000));

    expect((await store.listSegments("a")).map((item) => item.segmentId)).toEqual([
      "early",
      "late",
    ]);
    expect((await store.listSegments("b")).map((item) => item.segmentId)).toEqual(["other"]);
    expect(await store.listSegments("none")).toEqual([]);
  });

  it("重复登记同一段会更新它", async () => {
    const store = await create();
    await store.putSegment(segment("a", "s1", 1000));
    await store.putSegment({ ...segment("a", "s1", 1000), totalChunks: 7 });
    expect(await store.listSegments("a")).toEqual([
      { ...segment("a", "s1", 1000), totalChunks: 7 },
    ]);
  });

  it("分片按序号排序，清单只带序号和大小", async () => {
    const store = await create();
    for (const seq of [2, 0, 10, 1]) {
      await store.putChunk({
        sessionKey: "a",
        segmentId: "s1",
        seq,
        bytes: bytes("x".repeat(seq + 1)),
        sha256: "h",
      });
    }
    await store.putChunk({
      sessionKey: "a",
      segmentId: "s2",
      seq: 0,
      bytes: bytes("other"),
      sha256: "h",
    });

    expect(await store.listChunks("a", "s1")).toEqual([
      { seq: 0, size: 1 },
      { seq: 1, size: 2 },
      { seq: 2, size: 3 },
      { seq: 10, size: 11 },
    ]);
  });

  it("读回的内容和写入的一致", async () => {
    const store = await create();
    const content = bytes("第一片录制内容");
    await store.putChunk({
      sessionKey: "a",
      segmentId: "s1",
      seq: 0,
      bytes: content,
      sha256: "abc",
    });

    const chunk = await store.getChunk({ sessionKey: "a", segmentId: "s1", seq: 0 });
    expect(chunk?.sha256).toBe("abc");
    expect(new TextDecoder().decode(chunk?.bytes)).toBe("第一片录制内容");
    expect(await store.getChunk({ sessionKey: "a", segmentId: "s1", seq: 1 })).toBeUndefined();
  });

  it("删除分片和段", async () => {
    const store = await create();
    await store.putSegment(segment("a", "s1", 1000));
    await store.putChunk({
      sessionKey: "a",
      segmentId: "s1",
      seq: 0,
      bytes: bytes("x"),
      sha256: "h",
    });
    await store.putChunk({
      sessionKey: "a",
      segmentId: "s1",
      seq: 1,
      bytes: bytes("y"),
      sha256: "h",
    });

    await store.deleteChunk({ sessionKey: "a", segmentId: "s1", seq: 0 });
    expect(await store.listChunks("a", "s1")).toEqual([{ seq: 1, size: 1 }]);
    expect(await store.getChunk({ sessionKey: "a", segmentId: "s1", seq: 0 })).toBeUndefined();

    await store.deleteSegment("a", "s1");
    expect(await store.listSegments("a")).toEqual([]);
    // 删不存在的东西不报错
    await store.deleteChunk({ sessionKey: "a", segmentId: "s1", seq: 99 });
    await store.deleteSegment("a", "nope");
  });
});

describe("IndexedDbChunkStore", () => {
  it("关掉再打开，数据还在（模拟刷新页面）", async () => {
    const factory = new IDBFactory();
    const first = await IndexedDbChunkStore.open(factory);
    await first.putSegment(segment("a", "s1", 1000));
    await first.putChunk({
      sessionKey: "a",
      segmentId: "s1",
      seq: 0,
      bytes: bytes("还在"),
      sha256: "h",
    });
    first.close();

    const second = await IndexedDbChunkStore.open(factory);
    expect(await second.listSegments("a")).toHaveLength(1);
    const chunk = await second.getChunk({ sessionKey: "a", segmentId: "s1", seq: 0 });
    expect(new TextDecoder().decode(chunk?.bytes)).toBe("还在");
  });
});
