/**
 * 让一个页面只看见本机队列里归它管的录像。
 *
 * 同一个浏览器里的几个标签页共用一份队列。不分开的话，后开的页面会把先开的页面正在录的那一段
 * 当成「上次没录完的」提前收尾（之后录的分片就传不上去了），两边还会各传一遍同样的分片。
 *
 * 归这个页面管的段：它自己录的，和录它的页面已经不在了的（刷新、崩溃留下的；没有记是谁录的旧数据也算）。
 * 后一种会被改记到这个页面名下，免得别的页面也来接手。别的还开着的页面录的段，这里一概看不见。
 */
import type { ChunkInfo, ChunkKey, ChunkStore, SegmentMeta, StoredChunk } from "./chunk-store";

export interface OwnedChunkStoreOptions {
  /** 这个页面的标识。 */
  pageId: string;
  /** 那个页面还开着吗。 */
  isPageOpen: (pageId: string) => Promise<boolean>;
}

export class OwnedChunkStore implements ChunkStore {
  private readonly pageId: string;
  private readonly isPageOpen: (pageId: string) => Promise<boolean>;

  constructor(
    private readonly inner: ChunkStore,
    options: OwnedChunkStoreOptions,
  ) {
    this.pageId = options.pageId;
    this.isPageOpen = options.isPageOpen;
  }

  putSegment(meta: SegmentMeta): Promise<void> {
    return this.inner.putSegment({ ...meta, owner: this.pageId });
  }

  async listSegments(sessionKey: string): Promise<SegmentMeta[]> {
    const mine: SegmentMeta[] = [];
    // 一个页面问一次就够了
    const open = new Map<string, boolean>();
    for (const segment of await this.inner.listSegments(sessionKey)) {
      const { owner } = segment;
      if (owner === this.pageId) {
        mine.push(segment);
        continue;
      }
      if (owner !== undefined) {
        if (!open.has(owner)) open.set(owner, await this.isPageOpen(owner));
        if (open.get(owner)) continue;
      }
      const adopted = { ...segment, owner: this.pageId };
      await this.inner.putSegment(adopted);
      mine.push(adopted);
    }
    return mine;
  }

  /** 清理旧录像用的，要看见这台设备上所有的段。 */
  listAllSegments(): Promise<SegmentMeta[]> {
    return this.inner.listAllSegments();
  }

  deleteSegment(sessionKey: string, segmentId: string): Promise<void> {
    return this.inner.deleteSegment(sessionKey, segmentId);
  }

  discardSegment(sessionKey: string, segmentId: string): Promise<void> {
    return this.inner.discardSegment(sessionKey, segmentId);
  }

  putChunk(chunk: StoredChunk): Promise<void> {
    return this.inner.putChunk(chunk);
  }

  listChunks(sessionKey: string, segmentId: string): Promise<ChunkInfo[]> {
    return this.inner.listChunks(sessionKey, segmentId);
  }

  getChunk(key: ChunkKey): Promise<StoredChunk | undefined> {
    return this.inner.getChunk(key);
  }

  deleteChunk(key: ChunkKey): Promise<void> {
    return this.inner.deleteChunk(key);
  }
}
