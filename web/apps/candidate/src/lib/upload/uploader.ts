/**
 * 把本地队列里的录制分片可靠地传到后端。
 *
 * 保证：
 * - 分片只有在服务端确认后才从本地删除。
 * - 断网、超时、服务端 5xx 都会自动重试，间隔逐步拉长；恢复后接着传，不需要候选人操作。
 * - 页面刷新后重新启动时，先问服务端已经收到哪些，跳过已传的。
 * - 同一个分片重复上传不会出问题（服务端按段和序号做幂等）。
 *
 * 不碰浏览器接口：网络请求、等待、是否在线都从外面传进来，方便单测。
 */
import type { ChunkStore, RecordingTrack, SegmentMeta } from "./chunk-store";

export interface SegmentRef {
  track: RecordingTrack;
  segmentId: string;
}

/** 上传用到的后端接口。实现见 api/room-api.ts。 */
export interface UploadApi {
  /** 登记一段录制。重复调用结果相同。 */
  openSegment(segment: SegmentRef & { mimeType: string; startMs: number }): Promise<void>;
  /** 服务端已经收到的分片序号。 */
  receivedChunks(segment: SegmentRef): Promise<number[]>;
  /** 上传一个分片。重复上传相同内容算成功。 */
  putChunk(segment: SegmentRef, seq: number, bytes: ArrayBuffer, sha256: string): Promise<void>;
  /** 这一段传完了，告诉服务端一共多少个分片。 */
  completeSegment(segment: SegmentRef, totalChunks: number): Promise<void>;
}

/** 接口实现用它告诉上传器：这个错误重试有没有用。 */
export class UploadError extends Error {
  constructor(
    message: string,
    /** true：网络问题、超时、服务端临时故障，重试可能成功。false：链接失效、内容被拒，重试没用。 */
    readonly retryable: boolean,
    readonly status?: number,
  ) {
    super(message);
    this.name = "UploadError";
  }
}

export type UploadState =
  /** 本地没有待传的分片。 */
  | "idle"
  | "uploading"
  /** 上次请求失败，等一会儿再试。 */
  | "retrying"
  /** 浏览器报告没有网络，等恢复。 */
  | "offline"
  /** 遇到重试也没用的错误，已停止。 */
  | "failed";

export interface UploadStatus {
  state: UploadState;
  /** 还留在本地的分片数。 */
  pendingChunks: number;
  /** 还留在本地的字节数。 */
  pendingBytes: number;
  /** 本次启动以来已确认上传的字节数。 */
  uploadedBytes: number;
  /** 连续失败了几次；成功一次就清零。 */
  consecutiveFailures: number;
  lastError: string | null;
}

export interface UploaderOptions {
  sessionKey: string;
  store: ChunkStore;
  api: UploadApi;
  /** 等待若干毫秒。测试里换成假的。 */
  sleep?: (ms: number) => Promise<void>;
  /** 浏览器是否认为有网络。 */
  isOnline?: () => boolean;
  /** 第一次重试前等多久（毫秒）。 */
  baseDelayMs?: number;
  /** 重试间隔的上限（毫秒）。 */
  maxDelayMs?: number;
  /** 0 到 1 的随机数，用来给重试间隔加抖动。 */
  random?: () => number;
}

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** 第 n 次连续失败后等多久：指数增长，封顶，再乘上 0.5 到 1 的随机系数避免齐步重试。 */
export function retryDelayMs(failures: number, baseDelayMs: number, maxDelayMs: number, random: number): number {
  const exponential = Math.min(maxDelayMs, baseDelayMs * 2 ** Math.max(0, failures - 1));
  return Math.round(exponential * (0.5 + random / 2));
}

export class Uploader {
  private readonly sessionKey: string;
  private readonly store: ChunkStore;
  private readonly api: UploadApi;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly isOnline: () => boolean;
  private readonly baseDelayMs: number;
  private readonly maxDelayMs: number;
  private readonly random: () => number;

  private status: UploadStatus = {
    state: "idle",
    pendingChunks: 0,
    pendingBytes: 0,
    uploadedBytes: 0,
    consecutiveFailures: 0,
    lastError: null,
  };
  private readonly listeners = new Set<(status: UploadStatus) => void>();
  /** 已向服务端登记并查过已收序号的段。 */
  private readonly opened = new Map<string, Set<number>>();
  private running = false;
  private stopped = false;
  private wake: (() => void) | null = null;
  private dirty = false;
  private idleWaiters: Array<{ resolve: () => void; reject: (error: Error) => void }> = [];

  constructor(options: UploaderOptions) {
    this.sessionKey = options.sessionKey;
    this.store = options.store;
    this.api = options.api;
    this.sleep = options.sleep ?? defaultSleep;
    this.isOnline = options.isOnline ?? (() => true);
    this.baseDelayMs = options.baseDelayMs ?? 1_000;
    this.maxDelayMs = options.maxDelayMs ?? 30_000;
    this.random = options.random ?? Math.random;
  }

  getStatus(): UploadStatus {
    return this.status;
  }

  subscribe(listener: (status: UploadStatus) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** 开始工作。重复调用没有副作用。 */
  start(): void {
    if (this.running || this.stopped) return;
    this.running = true;
    void this.loop();
  }

  /** 停止。在途的请求会跑完，但不再发起新的。 */
  stop(): void {
    this.stopped = true;
    this.wakeUp();
  }

  /** 有新分片入队、段被关闭、网络恢复时调用，叫醒上传循环。 */
  notify(): void {
    this.dirty = true;
    this.wakeUp();
  }

  /** 等本地队列传空。遇到重试也没用的错误时抛出。 */
  drain(): Promise<void> {
    if (this.status.state === "failed") {
      return Promise.reject(new Error(this.status.lastError ?? "上传失败"));
    }
    return new Promise((resolve, reject) => {
      this.idleWaiters.push({ resolve, reject });
      this.notify();
    });
  }

  private wakeUp(): void {
    const wake = this.wake;
    this.wake = null;
    wake?.();
  }

  private waitForWake(): Promise<void> {
    return new Promise((resolve) => {
      this.wake = resolve;
    });
  }

  private setStatus(patch: Partial<UploadStatus>): void {
    this.status = { ...this.status, ...patch };
    for (const listener of this.listeners) listener(this.status);
  }

  private async refreshPending(): Promise<void> {
    let pendingChunks = 0;
    let pendingBytes = 0;
    for (const segment of await this.store.listSegments(this.sessionKey)) {
      for (const chunk of await this.store.listChunks(this.sessionKey, segment.segmentId)) {
        pendingChunks += 1;
        pendingBytes += chunk.size;
      }
    }
    this.setStatus({ pendingChunks, pendingBytes });
  }

  private async loop(): Promise<void> {
    while (!this.stopped) {
      this.dirty = false;

      if (!this.isOnline()) {
        this.setStatus({ state: "offline" });
        await this.waitForWake();
        continue;
      }

      try {
        const worked = await this.pass();
        if (this.stopped) break;
        if (worked || this.dirty) continue;
        // 这一轮什么都没做，而且期间没有新东西进来：队列空了
        this.setStatus({ state: "idle", consecutiveFailures: 0, lastError: null });
        for (const waiter of this.idleWaiters.splice(0)) waiter.resolve();
        await this.waitForWake();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (error instanceof UploadError && !error.retryable) {
          this.setStatus({ state: "failed", lastError: message });
          for (const waiter of this.idleWaiters.splice(0)) waiter.reject(new Error(message));
          this.running = false;
          return;
        }
        const failures = this.status.consecutiveFailures + 1;
        this.setStatus({ state: "retrying", consecutiveFailures: failures, lastError: message });
        const delay = retryDelayMs(failures, this.baseDelayMs, this.maxDelayMs, this.random());
        // 等待期间如果网络恢复或被 stop，提前醒来
        await Promise.race([this.sleep(delay), this.waitForWake()]);
      }
    }
    this.running = false;
  }

  /**
   * 把当前队列过一遍。返回 true 表示做了事（上传、清理或完成了一段），还需要再看一遍。
   * 任何一步失败都直接抛出，由 loop 决定重试还是放弃。
   */
  private async pass(): Promise<boolean> {
    let worked = false;
    const segments = await this.store.listSegments(this.sessionKey);
    await this.refreshPending();

    for (const segment of segments) {
      if (this.stopped) return worked;
      const ref: SegmentRef = { track: segment.track, segmentId: segment.segmentId };
      const received = await this.ensureOpened(segment);

      for (const info of await this.store.listChunks(this.sessionKey, segment.segmentId)) {
        if (this.stopped) return worked;
        const key = { sessionKey: this.sessionKey, segmentId: segment.segmentId, seq: info.seq };
        if (!received.has(info.seq)) {
          const chunk = await this.store.getChunk(key);
          // 列清单之后被别处删掉了（例如另一个标签页），跳过
          if (!chunk) continue;
          this.setStatus({ state: "uploading" });
          await this.api.putChunk(ref, info.seq, chunk.bytes, chunk.sha256);
          received.add(info.seq);
        }
        await this.store.deleteChunk(key);
        worked = true;
        this.setStatus({
          uploadedBytes: this.status.uploadedBytes + info.size,
          pendingChunks: Math.max(0, this.status.pendingChunks - 1),
          pendingBytes: Math.max(0, this.status.pendingBytes - info.size),
          consecutiveFailures: 0,
          lastError: null,
        });
      }

      // 段已经停止录制，而且本地没有剩余分片：通知服务端合并，然后删除本地登记。
      // 这里重新读一次段的信息，因为上传期间录制可能刚好结束。
      const latest = (await this.store.listSegments(this.sessionKey)).find(
        (candidate) => candidate.segmentId === segment.segmentId,
      );
      if (latest && latest.totalChunks !== null) {
        const remaining = await this.store.listChunks(this.sessionKey, segment.segmentId);
        if (remaining.length === 0) {
          this.setStatus({ state: "uploading" });
          await this.api.completeSegment(ref, latest.totalChunks);
          await this.store.deleteSegment(this.sessionKey, segment.segmentId);
          this.opened.delete(segment.segmentId);
          worked = true;
        }
      }
    }
    return worked;
  }

  private async ensureOpened(segment: SegmentMeta): Promise<Set<number>> {
    const known = this.opened.get(segment.segmentId);
    if (known) return known;
    this.setStatus({ state: "uploading" });
    const ref: SegmentRef = { track: segment.track, segmentId: segment.segmentId };
    await this.api.openSegment({ ...ref, mimeType: segment.mimeType, startMs: segment.startMs });
    const received = new Set(await this.api.receivedChunks(ref));
    this.opened.set(segment.segmentId, received);
    return received;
  }
}
