import { describe, expect, it } from "vitest";
import { type ChunkStore, MemoryChunkStore, sha256Hex } from "../upload/chunk-store";
import { type RecorderLike, SegmentRecorder } from "./segment-recorder";

const SESSION = "session-a";

/** 假的 MediaRecorder：测试里手动吐分片。 */
class FakeRecorder implements RecorderLike {
  mimeType = "";
  state: RecorderLike["state"] = "inactive";
  ondataavailable: RecorderLike["ondataavailable"] = null;
  onstop: RecorderLike["onstop"] = null;
  onerror: RecorderLike["onerror"] = null;
  timeslice: number | undefined;
  /** stop 时最后吐出的内容；null 表示不吐。 */
  finalData: string | null = "tail";

  start(timesliceMs?: number): void {
    this.state = "recording";
    this.timeslice = timesliceMs;
    this.mimeType = "video/webm;codecs=vp8,opus";
  }

  stop(): void {
    this.state = "inactive";
    if (this.finalData !== null) this.emit(this.finalData);
    queueMicrotask(() => this.onstop?.());
  }

  /** 攒着还没吐出来的内容。 */
  buffered = "";

  requestData(): void {
    const data = this.buffered;
    this.buffered = "";
    this.emit(data);
  }

  emit(text: string): void {
    this.ondataavailable?.({ data: new Blob([text]) });
  }
}

function setup(store: ChunkStore = new MemoryChunkStore()) {
  const fake = new FakeRecorder();
  const errors: Error[] = [];
  let stored = 0;
  const recorder = new SegmentRecorder({
    sessionKey: SESSION,
    store,
    createRecorder: () => fake,
    timesliceMs: 2_000,
    onStored: () => {
      stored += 1;
    },
    onError: (error) => errors.push(error),
    timelineNow: () => 12_345,
    newId: () => "seg-1",
  });
  return { fake, recorder, store, errors, storedCount: () => stored };
}

async function text(store: ChunkStore, seq: number): Promise<string | undefined> {
  const chunk = await store.getChunk({ sessionKey: SESSION, segmentId: "seg-1", seq });
  return chunk ? new TextDecoder().decode(chunk.bytes) : undefined;
}

describe("SegmentRecorder", () => {
  it("开始时登记段：格式取浏览器实际使用的，分片总数未知", async () => {
    const { recorder, store, fake } = setup();
    const meta = await recorder.start();

    expect(fake.timeslice).toBe(2_000);
    expect(meta).toEqual({
      sessionKey: SESSION,
      track: "camera",
      segmentId: "seg-1",
      mimeType: "video/webm;codecs=vp8,opus",
      startMs: 12_345,
      totalChunks: null,
    });
    expect(await store.listSegments(SESSION)).toEqual([meta]);
  });

  it("分片按到达顺序编号并写入本地队列，带校验和", async () => {
    const { recorder, store, fake, storedCount } = setup();
    await recorder.start();
    fake.emit("aaa");
    fake.emit("bb");
    await recorder.stop();

    expect(await store.listChunks(SESSION, "seg-1")).toEqual([
      { seq: 0, size: 3 },
      { seq: 1, size: 2 },
      { seq: 2, size: 4 },
    ]);
    expect(await text(store, 0)).toBe("aaa");
    expect(await text(store, 2)).toBe("tail");
    const first = await store.getChunk({ sessionKey: SESSION, segmentId: "seg-1", seq: 0 });
    expect(first?.sha256).toBe(
      await sha256Hex(new TextEncoder().encode("aaa").buffer as ArrayBuffer),
    );
    // 三个分片各通知一次，段结束再通知一次
    expect(storedCount()).toBe(4);
  });

  it("停止后登记分片总数，包含停止时吐出的最后一片", async () => {
    const { recorder, store, fake } = setup();
    await recorder.start();
    fake.emit("a");
    await recorder.stop();

    expect((await store.listSegments(SESSION))[0]?.totalChunks).toBe(2);
    expect(fake.state).toBe("inactive");
  });

  it("空分片不占序号", async () => {
    const { recorder, store, fake } = setup();
    await recorder.start();
    fake.emit("a");
    fake.emit("");
    fake.emit("b");
    fake.finalData = null;
    await recorder.stop();

    expect((await store.listChunks(SESSION, "seg-1")).map((chunk) => chunk.seq)).toEqual([0, 1]);
    expect((await store.listSegments(SESSION))[0]?.totalChunks).toBe(2);
  });

  it("重复 stop 只生效一次", async () => {
    const { recorder, store, fake } = setup();
    await recorder.start();
    fake.emit("a");
    await Promise.all([recorder.stop(), recorder.stop()]);
    await recorder.stop();

    expect((await store.listSegments(SESSION))[0]?.totalChunks).toBe(2);
  });

  it("没开始就 stop 不报错；重复 start 报错", async () => {
    const { recorder } = setup();
    await recorder.stop();

    const second = setup().recorder;
    await second.start();
    await expect(second.start()).rejects.toThrow("已经开始");
  });

  it("某个分片写入失败：报告错误，后面的分片照常写入，序号不变", async () => {
    const store = new MemoryChunkStore();
    const original = store.putChunk.bind(store);
    store.putChunk = async (chunk) => {
      if (chunk.seq === 1) throw new Error("存储空间不足");
      await original(chunk);
    };
    const { recorder, fake, errors } = setup(store);
    await recorder.start();
    fake.emit("a");
    fake.emit("b");
    fake.emit("c");
    fake.finalData = null;
    await recorder.stop();

    expect(errors.map((error) => error.message)).toEqual(["存储空间不足"]);
    expect((await store.listChunks(SESSION, "seg-1")).map((chunk) => chunk.seq)).toEqual([0, 2]);
    expect((await store.listSegments(SESSION))[0]?.totalChunks).toBe(3);
  });

  it("flush 马上切出一片，序号接着排；没在录的时候什么也不做", async () => {
    const { recorder, fake, store } = setup();
    recorder.flush(); // 还没开始
    await recorder.start();
    fake.emit("a");
    fake.buffered = "答完这一题时攒着的";
    recorder.flush();
    fake.emit("c");
    fake.finalData = null;
    await recorder.stop();
    recorder.flush(); // 已经停了

    expect(await text(store, 1)).toBe("答完这一题时攒着的");
    expect((await store.listChunks(SESSION, "seg-1")).map((chunk) => chunk.seq)).toEqual([0, 1, 2]);
  });

  it("浏览器不支持马上切片，或者切的时候报错，都不影响录制", async () => {
    const { recorder, fake, store, errors } = setup();
    await recorder.start();
    fake.requestData = () => {
      throw new DOMException("not recording", "InvalidStateError");
    };
    recorder.flush();
    (fake as RecorderLike).requestData = undefined;
    recorder.flush();
    fake.emit("a");
    await recorder.stop();

    expect(errors).toEqual([]);
    expect((await store.listChunks(SESSION, "seg-1")).map((chunk) => chunk.seq)).toEqual([0, 1]);
  });

  it("录制器报错时转给 onError", async () => {
    const { recorder, fake, errors } = setup();
    await recorder.start();
    fake.onerror?.({ error: new Error("编码器崩了") });
    expect(errors.map((error) => error.message)).toEqual(["编码器崩了"]);
  });
});
