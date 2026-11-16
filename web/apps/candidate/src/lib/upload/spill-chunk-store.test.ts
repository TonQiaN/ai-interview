import { describe, expect, it } from "vitest";
import { MemoryChunkStore, type SegmentMeta, type StoredChunk, sha256Hex } from "./chunk-store";
import { SpillChunkStore } from "./spill-chunk-store";
import { type SegmentRef, type UploadApi, Uploader } from "./uploader";

const SESSION = "session-a";
const SEGMENT = "seg-1";

/** 本机的库：可以让写入失败，像磁盘满了那样。读和删照常。 */
class FullDisk extends MemoryChunkStore {
  full = false;
  writes = 0;
  deletes = 0;

  private check(): void {
    if (!this.full) return;
    throw new DOMException("磁盘满了", "QuotaExceededError");
  }

  override async putSegment(meta: SegmentMeta): Promise<void> {
    this.check();
    await super.putSegment(meta);
  }

  override async putChunk(chunk: StoredChunk): Promise<void> {
    this.writes += 1;
    this.check();
    await super.putChunk(chunk);
  }

  override async deleteChunk(key: { sessionKey: string; segmentId: string; seq: number }) {
    this.deletes += 1;
    await super.deleteChunk(key);
  }
}

const segment = (totalChunks: number | null = null): SegmentMeta => ({
  sessionKey: SESSION,
  track: "camera",
  segmentId: SEGMENT,
  mimeType: "video/webm;codecs=vp8,opus",
  startMs: 0,
  totalChunks,
});

async function chunk(seq: number, size = 8): Promise<StoredChunk> {
  const bytes = new Uint8Array(size).fill(seq + 1).buffer as ArrayBuffer;
  return { sessionKey: SESSION, segmentId: SEGMENT, seq, bytes, sha256: await sha256Hex(bytes) };
}

function setup(maxBytes?: number) {
  const disk = new FullDisk();
  const spills: string[] = [];
  const store = new SpillChunkStore(disk, {
    maxBytes,
    onSpill: (error) => spills.push(error.name),
  });
  return { disk, store, spills };
}

const seqs = async (store: SpillChunkStore) =>
  (await store.listChunks(SESSION, SEGMENT)).map((info) => info.seq);

describe("本机存不下时把分片留在内存里", () => {
  it("写得进去的时候和本机的库一样，内存里什么都不留", async () => {
    const { disk, store, spills } = setup();
    await store.putSegment(segment());
    await store.putChunk(await chunk(0));
    expect(await seqs(store)).toEqual([0]);
    expect(await disk.listChunks(SESSION, SEGMENT)).toHaveLength(1);
    expect(store.spilledBytes).toBe(0);
    expect(spills).toEqual([]);
  });

  it("写不进去的分片留在内存里，和本机的排在一起，读得到也删得掉", async () => {
    const { disk, store } = setup();
    await store.putChunk(await chunk(0));
    disk.full = true;
    await store.putChunk(await chunk(1));
    await store.putChunk(await chunk(2));
    disk.full = false;
    await store.putChunk(await chunk(3));

    expect(await seqs(store)).toEqual([0, 1, 2, 3]);
    expect((await disk.listChunks(SESSION, SEGMENT)).map((info) => info.seq)).toEqual([0, 3]);
    expect(store.spilledBytes).toBe(16);
    const kept = await store.getChunk({ sessionKey: SESSION, segmentId: SEGMENT, seq: 1 });
    expect(new Uint8Array(kept?.bytes ?? new ArrayBuffer(0))).toEqual(new Uint8Array(8).fill(2));

    const before = disk.deletes;
    await store.deleteChunk({ sessionKey: SESSION, segmentId: SEGMENT, seq: 1 });
    expect(await seqs(store)).toEqual([0, 2, 3]);
    expect(store.spilledBytes).toBe(8);
    // 只在内存里的那一片，不去动本机的库
    expect(disk.deletes).toBe(before);
    await store.deleteChunk({ sessionKey: SESSION, segmentId: SEGMENT, seq: 0 });
    expect(await seqs(store)).toEqual([2, 3]);
  });

  it("连着写不进去只报第一次，中间写进去过再报", async () => {
    const { disk, store, spills } = setup();
    disk.full = true;
    await store.putChunk(await chunk(0));
    await store.putChunk(await chunk(1));
    expect(spills).toEqual(["QuotaExceededError"]);
    disk.full = false;
    await store.putChunk(await chunk(2));
    disk.full = true;
    await store.putChunk(await chunk(3));
    expect(spills).toEqual(["QuotaExceededError", "QuotaExceededError"]);
  });

  it("内存也放不下了才丢，交出原来的错误；传走几片腾出地方以后又能放", async () => {
    const { disk, store } = setup(20);
    disk.full = true;
    await store.putChunk(await chunk(0));
    await store.putChunk(await chunk(1));
    await expect(store.putChunk(await chunk(2))).rejects.toMatchObject({
      name: "QuotaExceededError",
    });
    expect(await seqs(store)).toEqual([0, 1]);
    expect(store.spilledBytes).toBe(16);

    await store.deleteChunk({ sessionKey: SESSION, segmentId: SEGMENT, seq: 0 });
    await store.putChunk(await chunk(3));
    expect(await seqs(store)).toEqual([1, 3]);
  });

  it("段的登记写不进去也留着，之后写进去了以本机的为准", async () => {
    const { disk, store } = setup();
    disk.full = true;
    await store.putSegment(segment());
    expect(await store.listSegments(SESSION)).toEqual([segment()]);
    expect(await store.listAllSegments()).toEqual([segment()]);
    expect(await disk.listSegments(SESSION)).toEqual([]);

    // 录制停了，分片总数登记不进本机：内存里的要更新
    await store.putSegment(segment(3));
    expect((await store.listSegments(SESSION))[0]?.totalChunks).toBe(3);

    disk.full = false;
    await store.putSegment(segment(4));
    expect(await store.listSegments(SESSION)).toEqual([segment(4)]);
    expect(await disk.listSegments(SESSION)).toEqual([segment(4)]);

    await store.deleteSegment(SESSION, SEGMENT);
    expect(await store.listSegments(SESSION)).toEqual([]);
  });

  it("本机登记过的段后来更新不进去，以内存里新的为准", async () => {
    const { disk, store } = setup();
    await store.putSegment(segment());
    disk.full = true;
    await store.putSegment(segment(5));
    expect(await store.listSegments(SESSION)).toEqual([segment(5)]);
    expect(await store.listSegments("别的面试")).toEqual([]);
  });

  it("丢掉一段时，内存里和本机的一起丢", async () => {
    const { disk, store } = setup();
    await store.putSegment(segment());
    await store.putChunk(await chunk(0));
    disk.full = true;
    await store.putChunk(await chunk(1));
    await store.discardSegment(SESSION, SEGMENT);
    expect(await seqs(store)).toEqual([]);
    expect(store.spilledBytes).toBe(0);
    expect(await store.listSegments(SESSION)).toEqual([]);
    expect(await disk.listChunks(SESSION, SEGMENT)).toEqual([]);
  });

  it("本机一直存不下，上传器照样把整段按顺序传完", async () => {
    const { disk, store } = setup();
    disk.full = true;
    await store.putSegment(segment());
    for (const seq of [0, 1, 2]) await store.putChunk(await chunk(seq));
    await store.putSegment(segment(3));

    const uploaded: number[] = [];
    const completed: number[] = [];
    const api: UploadApi = {
      openSegment: async () => {},
      receivedChunks: async () => [],
      putChunk: async (_segment: SegmentRef, seq: number) => {
        uploaded.push(seq);
      },
      completeSegment: async (_segment: SegmentRef, totalChunks: number) => {
        completed.push(totalChunks);
      },
    };
    const uploader = new Uploader({ sessionKey: SESSION, store, api, sleep: async () => {} });
    uploader.start();
    await uploader.drain();
    uploader.stop();

    expect(uploaded).toEqual([0, 1, 2]);
    expect(completed).toEqual([3]);
    expect(store.spilledBytes).toBe(0);
    expect(await store.listSegments(SESSION)).toEqual([]);
  });
});
