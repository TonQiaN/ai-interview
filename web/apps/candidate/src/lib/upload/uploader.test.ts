import { describe, expect, it } from "vitest";
import { MemoryChunkStore, type SegmentMeta, sha256Hex } from "./chunk-store";
import {
  retryDelayMs,
  type SegmentRef,
  type UploadApi,
  UploadError,
  Uploader,
  type UploadStatus,
} from "./uploader";

const SESSION = "session-a";

/** 假后端：记住收到的东西，可以按需注入失败。 */
class FakeApi implements UploadApi {
  readonly opened: string[] = [];
  readonly chunks = new Map<string, Map<number, { bytes: ArrayBuffer; sha256: string }>>();
  readonly completed: Array<{ segmentId: string; totalChunks: number; acceptMissing?: boolean }> =
    [];
  readonly calls: string[] = [];
  /** 每次调用前执行，抛错就模拟失败。 */
  beforeCall: (call: string) => void | Promise<void> = () => {};

  async openSegment(segment: { segmentId: string }): Promise<void> {
    await this.record(`open:${segment.segmentId}`);
    if (!this.chunks.has(segment.segmentId)) this.chunks.set(segment.segmentId, new Map());
    this.opened.push(segment.segmentId);
  }

  async receivedChunks({ segmentId }: SegmentRef): Promise<number[]> {
    await this.record(`received:${segmentId}`);
    return [...(this.chunks.get(segmentId)?.keys() ?? [])];
  }

  async putChunk(
    { segmentId }: SegmentRef,
    seq: number,
    bytes: ArrayBuffer,
    sha256: string,
  ): Promise<void> {
    await this.record(`put:${segmentId}:${seq}`);
    this.chunks.get(segmentId)?.set(seq, { bytes, sha256 });
  }

  async completeSegment(
    { segmentId }: SegmentRef,
    totalChunks: number,
    options: { acceptMissing?: boolean } = {},
  ): Promise<void> {
    await this.record(options.acceptMissing ? `complete!:${segmentId}` : `complete:${segmentId}`);
    this.completed.push(
      options.acceptMissing
        ? { segmentId, totalChunks, acceptMissing: true }
        : { segmentId, totalChunks },
    );
  }

  private async record(call: string): Promise<void> {
    this.calls.push(call);
    await this.beforeCall(call);
  }

  seqs(segmentId: string): number[] {
    return [...(this.chunks.get(segmentId)?.keys() ?? [])].sort((a, b) => a - b);
  }
}

function segment(
  segmentId: string,
  startMs: number,
  totalChunks: number | null = null,
): SegmentMeta {
  return {
    sessionKey: SESSION,
    track: "camera",
    segmentId,
    mimeType: "video/webm;codecs=vp8,opus",
    startMs,
    totalChunks,
  };
}

async function addChunk(
  store: MemoryChunkStore,
  segmentId: string,
  seq: number,
  text = `chunk-${seq}`,
) {
  const bytes = new TextEncoder().encode(text).buffer as ArrayBuffer;
  await store.putChunk({
    sessionKey: SESSION,
    segmentId,
    seq,
    bytes,
    sha256: await sha256Hex(bytes),
  });
}

function setup(overrides: Partial<ConstructorParameters<typeof Uploader>[0]> = {}) {
  const store = new MemoryChunkStore();
  const api = new FakeApi();
  const sleeps: number[] = [];
  const uploader = new Uploader({
    sessionKey: SESSION,
    store,
    api,
    sleep: async (ms) => {
      sleeps.push(ms);
    },
    random: () => 1,
    ...overrides,
  });
  const states: UploadStatus["state"][] = [];
  uploader.subscribe((status) => {
    if (states.at(-1) !== status.state) states.push(status.state);
  });
  return { store, api, uploader, sleeps, states };
}

describe("retryDelayMs", () => {
  it("指数增长并封顶", () => {
    expect(retryDelayMs(1, 1000, 30_000, 1)).toBe(1000);
    expect(retryDelayMs(2, 1000, 30_000, 1)).toBe(2000);
    expect(retryDelayMs(5, 1000, 30_000, 1)).toBe(16_000);
    expect(retryDelayMs(10, 1000, 30_000, 1)).toBe(30_000);
  });

  it("抖动只会缩短，最多缩到一半", () => {
    expect(retryDelayMs(3, 1000, 30_000, 0)).toBe(2000);
    expect(retryDelayMs(3, 1000, 30_000, 0.5)).toBe(3000);
  });
});

describe("Uploader", () => {
  it("按序上传分片，确认后从本地删除，段结束后通知服务端合并", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 3));
    await addChunk(store, "seg-1", 0);
    await addChunk(store, "seg-1", 1);
    await addChunk(store, "seg-1", 2);

    uploader.start();
    await uploader.drain();

    expect(api.calls).toEqual([
      "open:seg-1",
      "received:seg-1",
      "put:seg-1:0",
      "put:seg-1:1",
      "put:seg-1:2",
      "complete:seg-1",
    ]);
    expect(api.completed).toEqual([{ segmentId: "seg-1", totalChunks: 3 }]);
    expect(await store.listChunks(SESSION, "seg-1")).toEqual([]);
    expect(await store.listSegments(SESSION)).toEqual([]);
    expect(uploader.getStatus()).toMatchObject({
      state: "idle",
      pendingChunks: 0,
      pendingBytes: 0,
    });
    expect(uploader.getStatus().uploadedBytes).toBe("chunk-0".length * 3);
  });

  it("上传的内容和校验和与本地一致", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0, "你好，面试官");

    uploader.start();
    await uploader.drain();

    const received = api.chunks.get("seg-1")?.get(0);
    expect(new TextDecoder().decode(received?.bytes)).toBe("你好，面试官");
    expect(received?.sha256).toBe(
      await sha256Hex(new TextEncoder().encode("你好，面试官").buffer as ArrayBuffer),
    );
  });

  it("请求失败会重试，间隔逐步拉长，成功前分片一直留在本地", async () => {
    const { store, api, uploader, sleeps, states } = setup();
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0);

    let failures = 3;
    api.beforeCall = async (call) => {
      if (call.startsWith("put:") && failures > 0) {
        failures -= 1;
        expect(await store.listChunks(SESSION, "seg-1")).toHaveLength(1);
        throw new UploadError("网络错误", true);
      }
    };

    uploader.start();
    await uploader.drain();

    expect(sleeps).toEqual([1000, 2000, 4000]);
    expect(api.seqs("seg-1")).toEqual([0]);
    expect(states).toContain("retrying");
    expect(uploader.getStatus()).toMatchObject({
      state: "idle",
      consecutiveFailures: 0,
      lastError: null,
    });
  });

  it("不是 UploadError 的异常（例如 fetch 抛的 TypeError）也按可重试处理", async () => {
    const { store, api, uploader, sleeps } = setup();
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0);

    let thrown = false;
    api.beforeCall = (call) => {
      if (call.startsWith("open:") && !thrown) {
        thrown = true;
        throw new TypeError("Failed to fetch");
      }
    };

    uploader.start();
    await uploader.drain();
    expect(sleeps).toEqual([1000]);
    expect(api.completed).toHaveLength(1);
  });

  it("刷新后重新启动：服务端已有的分片不再上传，直接从本地清掉", async () => {
    const { store, api, uploader } = setup();
    // 服务端已经有 0 和 1
    api.chunks.set(
      "seg-1",
      new Map([
        [0, { bytes: new ArrayBuffer(1), sha256: "x" }],
        [1, { bytes: new ArrayBuffer(1), sha256: "x" }],
      ]),
    );
    await store.putSegment(segment("seg-1", 1000, 4));
    for (const seq of [0, 1, 2, 3]) await addChunk(store, "seg-1", seq);

    uploader.start();
    await uploader.drain();

    expect(api.calls.filter((call) => call.startsWith("put:"))).toEqual([
      "put:seg-1:2",
      "put:seg-1:3",
    ]);
    expect(await store.listChunks(SESSION, "seg-1")).toEqual([]);
    expect(api.completed).toEqual([{ segmentId: "seg-1", totalChunks: 4 }]);
  });

  it("多段按开始时间依次上传：上次没传完的旧段先传", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-new", 5000, 1));
    await store.putSegment(segment("seg-old", 1000, 2));
    await addChunk(store, "seg-new", 0);
    await addChunk(store, "seg-old", 0);
    await addChunk(store, "seg-old", 1);

    uploader.start();
    await uploader.drain();

    expect(
      api.calls.filter((call) => call.startsWith("put:") || call.startsWith("complete:")),
    ).toEqual([
      "put:seg-old:0",
      "put:seg-old:1",
      "complete:seg-old",
      "put:seg-new:0",
      "complete:seg-new",
    ]);
  });

  it("还在录的段不会被通知合并；录制结束后再通知", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000));
    await addChunk(store, "seg-1", 0);

    uploader.start();
    await uploader.drain();
    expect(api.seqs("seg-1")).toEqual([0]);
    expect(api.completed).toEqual([]);

    // 录制继续产生分片，然后停止
    await addChunk(store, "seg-1", 1);
    await store.putSegment(segment("seg-1", 1000, 2));
    uploader.notify();
    await uploader.drain();

    expect(api.seqs("seg-1")).toEqual([0, 1]);
    expect(api.completed).toEqual([{ segmentId: "seg-1", totalChunks: 2 }]);
    // 段只登记一次
    expect(api.opened).toEqual(["seg-1"]);
  });

  it("上传过程中入队的新分片也会被传走", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000));
    await addChunk(store, "seg-1", 0);

    let added = false;
    api.beforeCall = async (call) => {
      if (call === "put:seg-1:0" && !added) {
        added = true;
        await addChunk(store, "seg-1", 1);
        uploader.notify();
      }
    };

    uploader.start();
    await uploader.drain();
    expect(api.seqs("seg-1")).toEqual([0, 1]);
  });

  it("传得比录得慢：新录下的分片马上算进还没传完的里面，不等手上这一轮传完", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 5));
    for (const seq of [0, 1, 2]) await addChunk(store, "seg-1", seq);

    // 每传一片之前看一眼还剩几片；传第 0 片的时候又录下了两片
    const pending: number[] = [];
    api.beforeCall = async (call) => {
      if (!call.startsWith("put:")) return;
      if (call === "put:seg-1:0") {
        await addChunk(store, "seg-1", 3);
        uploader.notify();
        await addChunk(store, "seg-1", 4);
        uploader.notify();
      }
      await new Promise((resolve) => setTimeout(resolve, 0));
      pending.push(uploader.getStatus().pendingChunks);
    };

    uploader.start();
    await uploader.drain();
    // 一直是本机真的留着的片数，只减不增；以前是 3、2、1 传完了又跳回 2
    expect(pending).toEqual([5, 4, 3, 2, 1]);
    expect(uploader.getStatus().pendingBytes).toBe(0);
    expect(api.seqs("seg-1")).toEqual([0, 1, 2, 3, 4]);
  });

  it("断网的时候录下的分片也算进还没传完的里面", async () => {
    const { store, uploader } = setup({ isOnline: () => false });
    await store.putSegment(segment("seg-1", 1000));
    uploader.start();

    for (const seq of [0, 1, 2]) {
      await addChunk(store, "seg-1", seq);
      uploader.notify();
    }
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(uploader.getStatus().state).toBe("offline");
    expect(uploader.getStatus().pendingChunks).toBe(3);
    expect(uploader.getStatus().pendingBytes).toBeGreaterThan(0);
    uploader.stop();
  });

  it("没有网络时等待，恢复后继续", async () => {
    let online = false;
    const { store, api, uploader, states } = setup({ isOnline: () => online });
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0);

    uploader.start();
    await Promise.resolve();
    expect(uploader.getStatus().state).toBe("offline");
    expect(api.calls).toEqual([]);

    online = true;
    uploader.notify();
    await uploader.drain();
    expect(api.completed).toHaveLength(1);
    expect(states).toEqual(["offline", "uploading", "idle"]);
  });

  it("重试也没用的错误（链接失效等）：停止上传，分片留在本地，drain 报错", async () => {
    const { store, api, uploader, sleeps } = setup();
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0);
    api.beforeCall = (call) => {
      if (call.startsWith("put:")) throw new UploadError("链接已失效", false, 401);
    };

    uploader.start();
    await expect(uploader.drain()).rejects.toThrow("链接已失效");

    expect(sleeps).toEqual([]);
    expect(uploader.getStatus()).toMatchObject({
      state: "failed",
      lastError: "链接已失效",
      pendingChunks: 1,
    });
    expect(await store.listChunks(SESSION, "seg-1")).toHaveLength(1);
    await expect(uploader.drain()).rejects.toThrow("链接已失效");
  });

  it("服务端已经不收这一段了：本机留着的丢掉，后面的段照常传", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 3));
    await addChunk(store, "seg-1", 1);
    await addChunk(store, "seg-1", 2);
    await store.putSegment(segment("seg-2", 5000, 1));
    await addChunk(store, "seg-2", 0);
    api.beforeCall = (call) => {
      if (call.startsWith("put:seg-1:")) {
        throw new UploadError("这一段已经合并，不能再传", false, 409, "segment_closed");
      }
    };

    uploader.start();
    await uploader.drain();

    // 第一片被拒之后不再试这一段剩下的，也不再通知合并
    expect(api.calls.filter((call) => call.includes("seg-1"))).toEqual([
      "open:seg-1",
      "received:seg-1",
      "put:seg-1:1",
    ]);
    expect(await store.listSegments(SESSION)).toEqual([]);
    expect(await store.listChunks(SESSION, "seg-1")).toEqual([]);
    expect(api.seqs("seg-2")).toEqual([0]);
    expect(api.completed).toEqual([{ segmentId: "seg-2", totalChunks: 1 }]);
    expect(uploader.getStatus()).toMatchObject({ state: "idle", pendingChunks: 0 });
  });

  it("同样是 409，别的原因照旧算传不上去", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 1));
    await addChunk(store, "seg-1", 0);
    api.beforeCall = (call) => {
      if (call.startsWith("put:")) throw new UploadError("面试还没开始", false, 409);
    };

    uploader.start();
    await expect(uploader.drain()).rejects.toThrow("面试还没开始");
    expect(await store.listChunks(SESSION, "seg-1")).toHaveLength(1);
  });

  it("服务端缺的分片本机也没有了：让服务端有多少合并多少，不卡在这一段", async () => {
    const { store, api, uploader } = setup();
    // 一共 3 片，第 1 片当时没存下来
    await store.putSegment(segment("seg-1", 1000, 3));
    await addChunk(store, "seg-1", 0);
    await addChunk(store, "seg-1", 2);
    api.beforeCall = (call) => {
      if (call === "complete:seg-1") {
        throw new UploadError("还缺 1 个分片", false, 409, "chunks_missing");
      }
    };

    uploader.start();
    await uploader.drain();

    expect(api.seqs("seg-1")).toEqual([0, 2]);
    expect(api.calls.slice(-2)).toEqual(["complete:seg-1", "complete!:seg-1"]);
    expect(api.completed).toEqual([{ segmentId: "seg-1", totalChunks: 3, acceptMissing: true }]);
    expect(await store.listSegments(SESSION)).toEqual([]);
    expect(uploader.getStatus().state).toBe("idle");
  });

  it("让服务端有多少合并多少的那一次请求断了：重试时再来一遍", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 2));
    await addChunk(store, "seg-1", 1);
    let dropped = false;
    api.beforeCall = (call) => {
      if (call === "complete:seg-1") {
        throw new UploadError("还缺 1 个分片", false, 409, "chunks_missing");
      }
      if (call === "complete!:seg-1" && !dropped) {
        dropped = true;
        throw new UploadError("网络连接失败", true);
      }
    };

    uploader.start();
    await uploader.drain();

    expect(api.completed).toEqual([{ segmentId: "seg-1", totalChunks: 2, acceptMissing: true }]);
    expect(await store.listSegments(SESSION)).toEqual([]);
  });

  it("只处理自己这场面试的分片", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment({ ...segment("other", 1000, 1), sessionKey: "session-b" });
    await store.putChunk({
      sessionKey: "session-b",
      segmentId: "other",
      seq: 0,
      bytes: new ArrayBuffer(4),
      sha256: "x",
    });

    uploader.start();
    await uploader.drain();
    expect(api.calls).toEqual([]);
    expect(await store.listChunks("session-b", "other")).toHaveLength(1);
  });

  it("stop 之后不再发起请求", async () => {
    const { store, api, uploader } = setup();
    await store.putSegment(segment("seg-1", 1000, 2));
    await addChunk(store, "seg-1", 0);
    await addChunk(store, "seg-1", 1);
    api.beforeCall = (call) => {
      if (call === "put:seg-1:0") uploader.stop();
    };

    uploader.start();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(api.calls.filter((call) => call.startsWith("put:"))).toEqual(["put:seg-1:0"]);
  });
});
