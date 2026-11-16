/**
 * 录制分片的本地队列。
 *
 * 分片录出来先落到这里，上传成功后才删除。这样断网、刷新、浏览器崩溃都不会丢录制：
 * 下次打开同一条面试链接时，从这里把没传完的继续传。
 *
 * 两个实现：
 * - MemoryChunkStore：放内存里，页面一关就没了。用于单测，以及 IndexedDB 不可用时的退路。
 * - IndexedDbChunkStore（idb-chunk-store.ts）：正式用的。
 */

/** 和后端 interview_recordings.track 的取值一致。 */
export type RecordingTrack = "camera" | "microphone" | "screen";

/** 一段连续的录制。刷新或崩溃后重新开始录会产生新的一段，每段是一个独立可播放的文件。 */
export interface SegmentMeta {
  /** 区分不同面试的本地键（由令牌的哈希得到，不是令牌本身）。 */
  sessionKey: string;
  /** 录的是什么。目前只有 camera（摄像头画面加混音音轨）。 */
  track: RecordingTrack;
  /** 段的标识，客户端生成的 UUID。离线时也能生成，服务端按它做幂等。 */
  segmentId: string;
  /** MediaRecorder 实际使用的格式，例如 video/webm;codecs=vp8,opus。 */
  mimeType: string;
  /** 这一段从面试时间轴的哪一毫秒开始（相对面试开始）。 */
  startMs: number;
  /** 这一段一共多少个分片。还在录的时候是 null，停止录制后写入。 */
  totalChunks: number | null;
  /**
   * 这一段是什么时候开始录的（Date.now()）。用来清掉放了太久、已经传不上去的录像。
   * 没有这一项的按很久以前录的算。
   */
  createdAt?: number;
  /**
   * 录这一段的页面的标识。同一个浏览器里几个标签页共用这份队列，靠它分清一段是谁的
   * （owned-chunk-store.ts）。没有这一项的是加这个标记之前留下的。
   */
  owner?: string;
}

export interface ChunkKey {
  sessionKey: string;
  segmentId: string;
  /** 分片在段内的序号，从 0 开始。 */
  seq: number;
}

export interface StoredChunk extends ChunkKey {
  bytes: ArrayBuffer;
  /** bytes 的 SHA-256，小写十六进制。 */
  sha256: string;
}

export interface ChunkInfo {
  seq: number;
  size: number;
}

export interface ChunkStore {
  /** 登记或更新一段。 */
  putSegment(meta: SegmentMeta): Promise<void>;
  /** 某场面试的所有段，按开始时间排序。 */
  listSegments(sessionKey: string): Promise<SegmentMeta[]>;
  /** 这台设备上所有面试留下的段。 */
  listAllSegments(): Promise<SegmentMeta[]>;
  /** 删除一段的登记。调用前应确保它的分片都已删除。 */
  deleteSegment(sessionKey: string, segmentId: string): Promise<void>;
  /** 把一段连同它还留在本地的分片一起删掉。传不上去的录像不留在设备上。 */
  discardSegment(sessionKey: string, segmentId: string): Promise<void>;

  putChunk(chunk: StoredChunk): Promise<void>;
  /** 某段还留在本地的分片，按序号排序。只返回序号和大小，不读内容。 */
  listChunks(sessionKey: string, segmentId: string): Promise<ChunkInfo[]>;
  getChunk(key: ChunkKey): Promise<StoredChunk | undefined>;
  deleteChunk(key: ChunkKey): Promise<void>;
}

function chunkId(key: ChunkKey): string {
  return `${key.sessionKey}/${key.segmentId}/${key.seq}`;
}

export class MemoryChunkStore implements ChunkStore {
  private readonly segments = new Map<string, SegmentMeta>();
  private readonly chunks = new Map<string, StoredChunk>();

  async putSegment(meta: SegmentMeta): Promise<void> {
    this.segments.set(`${meta.sessionKey}/${meta.segmentId}`, { ...meta });
  }

  async listSegments(sessionKey: string): Promise<SegmentMeta[]> {
    return [...this.segments.values()]
      .filter((segment) => segment.sessionKey === sessionKey)
      .sort((a, b) => a.startMs - b.startMs)
      .map((segment) => ({ ...segment }));
  }

  async listAllSegments(): Promise<SegmentMeta[]> {
    return [...this.segments.values()].map((segment) => ({ ...segment }));
  }

  async deleteSegment(sessionKey: string, segmentId: string): Promise<void> {
    this.segments.delete(`${sessionKey}/${segmentId}`);
  }

  async discardSegment(sessionKey: string, segmentId: string): Promise<void> {
    for (const [id, chunk] of this.chunks) {
      if (chunk.sessionKey === sessionKey && chunk.segmentId === segmentId) this.chunks.delete(id);
    }
    this.segments.delete(`${sessionKey}/${segmentId}`);
  }

  async putChunk(chunk: StoredChunk): Promise<void> {
    this.chunks.set(chunkId(chunk), chunk);
  }

  async listChunks(sessionKey: string, segmentId: string): Promise<ChunkInfo[]> {
    return [...this.chunks.values()]
      .filter((chunk) => chunk.sessionKey === sessionKey && chunk.segmentId === segmentId)
      .map((chunk) => ({ seq: chunk.seq, size: chunk.bytes.byteLength }))
      .sort((a, b) => a.seq - b.seq);
  }

  async getChunk(key: ChunkKey): Promise<StoredChunk | undefined> {
    return this.chunks.get(chunkId(key));
  }

  async deleteChunk(key: ChunkKey): Promise<void> {
    this.chunks.delete(chunkId(key));
  }
}

/** 计算 SHA-256，返回小写十六进制。 */
export async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}
