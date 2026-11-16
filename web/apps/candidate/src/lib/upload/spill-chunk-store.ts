/**
 * 本机存不下录像时的退路。
 *
 * 候选人的设备磁盘满了，或者浏览器给这个站点的配额用完了，分片就写不进 IndexedDB。
 * 以前这一片直接丢掉，录像中间出现缺口，缺口之后的内容服务端合并不出来。
 * 这里把写不进去的分片先留在内存里：上传器照样能读到、传走、传完删掉。网络是通的时候内存里
 * 只会有一两片；断网又存不下的时候会越积越多，所以有上限，超过上限才真的丢。
 *
 * 留在内存里的分片页面一刷新就没了，这个避免不了。
 */
import type { ChunkInfo, ChunkKey, ChunkStore, SegmentMeta, StoredChunk } from "./chunk-store";

/** 内存里最多留这么多字节的分片。默认码率下大约是一刻钟的录像。 */
export const DEFAULT_SPILL_BYTES = 64 * 1024 * 1024;

export interface SpillChunkStoreOptions {
  /** 内存里最多留多少字节，默认 [`DEFAULT_SPILL_BYTES`]。 */
  maxBytes?: number;
  /**
   * 本机开始存不下了（之前一直写得进去，这一片写不进去、改放内存）。
   * 连着写不进去只报第一次；中间又写进去过，下次再写不进去会再报。
   */
  onSpill?: (error: Error) => void;
}

const chunkId = (key: ChunkKey): string => `${key.sessionKey}/${key.segmentId}/${key.seq}`;
const segmentId = (sessionKey: string, segment: string): string => `${sessionKey}/${segment}`;

const asError = (error: unknown): Error =>
  error instanceof Error ? error : new Error(String(error));

export class SpillChunkStore implements ChunkStore {
  private readonly maxBytes: number;
  private readonly chunks = new Map<string, StoredChunk>();
  private readonly segments = new Map<string, SegmentMeta>();
  private bytes = 0;
  private spilling = false;

  constructor(
    private readonly primary: ChunkStore,
    private readonly options: SpillChunkStoreOptions = {},
  ) {
    this.maxBytes = options.maxBytes ?? DEFAULT_SPILL_BYTES;
  }

  /** 现在有多少字节的分片只在内存里。 */
  get spilledBytes(): number {
    return this.bytes;
  }

  async putSegment(meta: SegmentMeta): Promise<void> {
    const id = segmentId(meta.sessionKey, meta.segmentId);
    try {
      await this.primary.putSegment(meta);
      this.segments.delete(id);
    } catch {
      // 段的登记只有几十个字节，写不进去也留在内存里，不然这一段的分片没人传
      this.segments.set(id, { ...meta });
    }
  }

  async listSegments(sessionKey: string): Promise<SegmentMeta[]> {
    const stored = await this.primary.listSegments(sessionKey);
    return this.withSpilledSegments(stored, (meta) => meta.sessionKey === sessionKey).sort(
      (a, b) => a.startMs - b.startMs,
    );
  }

  async listAllSegments(): Promise<SegmentMeta[]> {
    return this.withSpilledSegments(await this.primary.listAllSegments(), () => true);
  }

  async deleteSegment(sessionKey: string, segment: string): Promise<void> {
    this.segments.delete(segmentId(sessionKey, segment));
    await this.primary.deleteSegment(sessionKey, segment);
  }

  async discardSegment(sessionKey: string, segment: string): Promise<void> {
    for (const [id, chunk] of this.chunks) {
      if (chunk.sessionKey === sessionKey && chunk.segmentId === segment) this.forget(id);
    }
    this.segments.delete(segmentId(sessionKey, segment));
    await this.primary.discardSegment(sessionKey, segment);
  }

  async putChunk(chunk: StoredChunk): Promise<void> {
    const id = chunkId(chunk);
    try {
      await this.primary.putChunk(chunk);
      this.forget(id);
      this.spilling = false;
      return;
    } catch (cause) {
      const error = asError(cause);
      this.forget(id);
      // 内存也放不下了：这一片只能丢。把原来的错误交出去，调用方照旧记一条录制出错
      if (this.bytes + chunk.bytes.byteLength > this.maxBytes) throw error;
      this.chunks.set(id, chunk);
      this.bytes += chunk.bytes.byteLength;
      if (!this.spilling) {
        this.spilling = true;
        this.options.onSpill?.(error);
      }
    }
  }

  async listChunks(sessionKey: string, segment: string): Promise<ChunkInfo[]> {
    const bySeq = new Map<number, ChunkInfo>();
    for (const info of await this.primary.listChunks(sessionKey, segment)) {
      bySeq.set(info.seq, info);
    }
    for (const chunk of this.chunks.values()) {
      if (chunk.sessionKey === sessionKey && chunk.segmentId === segment) {
        bySeq.set(chunk.seq, { seq: chunk.seq, size: chunk.bytes.byteLength });
      }
    }
    return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
  }

  async getChunk(key: ChunkKey): Promise<StoredChunk | undefined> {
    return this.chunks.get(chunkId(key)) ?? (await this.primary.getChunk(key));
  }

  async deleteChunk(key: ChunkKey): Promise<void> {
    // 只在内存里的不用去动本机的库：它正存不下，没必要再多一次会失败的写
    if (this.forget(chunkId(key))) return;
    await this.primary.deleteChunk(key);
  }

  private forget(id: string): boolean {
    const chunk = this.chunks.get(id);
    if (!chunk) return false;
    this.chunks.delete(id);
    this.bytes -= chunk.bytes.byteLength;
    return true;
  }

  /** 内存里的登记比本机库里的新（库里那份没能更新），同一段以内存里的为准。 */
  private withSpilledSegments(
    stored: SegmentMeta[],
    wanted: (meta: SegmentMeta) => boolean,
  ): SegmentMeta[] {
    const merged = new Map(
      stored.map((meta) => [segmentId(meta.sessionKey, meta.segmentId), meta]),
    );
    for (const [id, meta] of this.segments) {
      if (wanted(meta)) merged.set(id, { ...meta });
    }
    return [...merged.values()];
  }
}
