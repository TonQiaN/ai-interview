/**
 * 录一段：把 MediaRecorder 吐出来的分片按顺序写进本地队列。
 *
 * 一个 SegmentRecorder 对应一段连续的录制。页面刷新或崩溃后重新开始录，就新建一个，
 * 得到新的段。上传由 Uploader 负责，这里只管「录出来的每个字节都落到本地队列」。
 *
 * MediaRecorder 的构造从外面传进来，单测里换成假的。
 */
import type { ChunkStore, RecordingTrack, SegmentMeta } from "../upload/chunk-store";
import { sha256Hex } from "../upload/chunk-store";

/** 这里用到的 MediaRecorder 的那一小部分。 */
export interface RecorderLike {
  readonly mimeType: string;
  readonly state: "inactive" | "recording" | "paused";
  ondataavailable: ((event: { data: Blob }) => void) | null;
  onstop: (() => void) | null;
  onerror: ((event: unknown) => void) | null;
  start(timesliceMs?: number): void;
  stop(): void;
}

export interface SegmentRecorderOptions {
  sessionKey: string;
  /** 录的是什么，默认 camera。 */
  track?: RecordingTrack;
  store: ChunkStore;
  /** 创建底层录制器。正式环境是 new MediaRecorder(stream, options)。 */
  createRecorder: () => RecorderLike;
  /** 每隔多久切一个分片（毫秒）。 */
  timesliceMs?: number;
  /** 每写入一个分片、以及段结束时调用，用来叫醒上传器。 */
  onStored?: () => void;
  /** 录制器报错或分片写入失败时调用。录制会尽量继续。 */
  onError?: (error: Error) => void;
  /** 面试时间轴上的当前时刻（相对面试开始的毫秒）。 */
  timelineNow: () => number;
  /** 生成段标识。 */
  newId?: () => string;
}

export class SegmentRecorder {
  readonly segmentId: string;
  private readonly options: SegmentRecorderOptions;
  private recorder: RecorderLike | null = null;
  private meta: SegmentMeta | null = null;
  private nextSeq = 0;
  /** 分片写入排成一条链，保证按序号落盘，停止时能等它们全部写完。 */
  private writes: Promise<void> = Promise.resolve();
  private stopping: Promise<void> | null = null;

  constructor(options: SegmentRecorderOptions) {
    this.options = options;
    this.segmentId = (options.newId ?? (() => crypto.randomUUID()))();
  }

  /** 这一段从面试时间轴的哪一毫秒开始；还没开始是 null。 */
  get startMs(): number | null {
    return this.meta?.startMs ?? null;
  }

  get mimeType(): string | null {
    return this.meta?.mimeType ?? null;
  }

  async start(): Promise<SegmentMeta> {
    if (this.recorder) throw new Error("这一段已经开始录制");
    const { options } = this;
    const recorder = options.createRecorder();
    this.recorder = recorder;

    recorder.ondataavailable = (event) => {
      // 有的浏览器会吐出空分片，跳过，不占序号
      if (!event.data || event.data.size === 0) return;
      const seq = this.nextSeq;
      this.nextSeq += 1;
      this.writes = this.writes.then(() => this.storeChunk(seq, event.data));
    };
    recorder.onerror = (event) => {
      const cause = typeof event === "object" && event !== null && "error" in event ? event.error : event;
      options.onError?.(cause instanceof Error ? cause : new Error("录制出错"));
    };

    recorder.start(options.timesliceMs ?? 5_000);
    const meta: SegmentMeta = {
      sessionKey: options.sessionKey,
      track: options.track ?? "camera",
      segmentId: this.segmentId,
      // start() 之后浏览器才确定实际使用的格式
      mimeType: recorder.mimeType,
      startMs: options.timelineNow(),
      totalChunks: null,
    };
    this.meta = meta;
    await options.store.putSegment(meta);
    return meta;
  }

  /**
   * 停止录制。等最后一个分片写进本地队列、段的分片总数登记好之后才返回。
   * 重复调用返回同一个结果。
   */
  stop(): Promise<void> {
    if (!this.stopping) this.stopping = this.doStop();
    return this.stopping;
  }

  private async doStop(): Promise<void> {
    const { recorder, meta, options } = this;
    if (!recorder || !meta) return;

    if (recorder.state !== "inactive") {
      await new Promise<void>((resolve) => {
        recorder.onstop = () => resolve();
        recorder.stop();
      });
    }
    // onstop 之前浏览器会先发出最后一个分片，这里等它写完
    await this.writes;

    this.meta = { ...meta, totalChunks: this.nextSeq };
    await options.store.putSegment(this.meta);
    options.onStored?.();
  }

  private async storeChunk(seq: number, blob: Blob): Promise<void> {
    const { options } = this;
    try {
      const bytes = await blob.arrayBuffer();
      await options.store.putChunk({
        sessionKey: options.sessionKey,
        segmentId: this.segmentId,
        seq,
        bytes,
        sha256: await sha256Hex(bytes),
      });
      options.onStored?.();
    } catch (error) {
      options.onError?.(error instanceof Error ? error : new Error(String(error)));
    }
  }
}
