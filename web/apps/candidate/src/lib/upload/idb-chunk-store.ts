/**
 * 用 IndexedDB 实现的分片队列。
 *
 * 分片内容存成 ArrayBuffer 而不是 Blob：部分 iOS WebView 里往 IndexedDB 存 Blob 会失败或读回空内容。
 */
import type { ChunkInfo, ChunkKey, ChunkStore, SegmentMeta, StoredChunk } from "./chunk-store";

const DB_NAME = "ai-interview-recording";
const DB_VERSION = 1;
const SEGMENTS = "segments";
const CHUNKS = "chunks";
/** 分片的元信息（不含内容）另存一份，列清单时不用把几百 KB 的内容读出来。 */
const CHUNK_INDEX = "chunk-index";

function requestToPromise<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB 请求失败"));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB 事务失败"));
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB 事务被中止"));
  });
}

export class IndexedDbChunkStore implements ChunkStore {
  private constructor(private readonly db: IDBDatabase) {}

  /**
   * 打开数据库。IndexedDB 不可用（隐私模式、被禁用、配额为零）时会抛错，
   * 调用方应退回 MemoryChunkStore 并告诉候选人不要刷新页面。
   */
  static async open(factory: IDBFactory = indexedDB): Promise<IndexedDbChunkStore> {
    const request = factory.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(SEGMENTS)) {
        db.createObjectStore(SEGMENTS, { keyPath: ["sessionKey", "segmentId"] });
      }
      if (!db.objectStoreNames.contains(CHUNKS)) {
        db.createObjectStore(CHUNKS, { keyPath: ["sessionKey", "segmentId", "seq"] });
      }
      if (!db.objectStoreNames.contains(CHUNK_INDEX)) {
        db.createObjectStore(CHUNK_INDEX, { keyPath: ["sessionKey", "segmentId", "seq"] });
      }
    };
    const db = await requestToPromise(request);
    // 别的标签页升级数据库时让路，避免把对方卡住
    db.onversionchange = () => db.close();
    return new IndexedDbChunkStore(db);
  }

  close(): void {
    this.db.close();
  }

  async putSegment(meta: SegmentMeta): Promise<void> {
    const transaction = this.db.transaction(SEGMENTS, "readwrite");
    transaction.objectStore(SEGMENTS).put({ ...meta });
    await transactionDone(transaction);
  }

  async listSegments(sessionKey: string): Promise<SegmentMeta[]> {
    const transaction = this.db.transaction(SEGMENTS, "readonly");
    const range = IDBKeyRange.bound([sessionKey, ""], [sessionKey, "￿"]);
    const rows = (await requestToPromise(transaction.objectStore(SEGMENTS).getAll(range))) as SegmentMeta[];
    return rows.sort((a, b) => a.startMs - b.startMs);
  }

  async deleteSegment(sessionKey: string, segmentId: string): Promise<void> {
    const transaction = this.db.transaction(SEGMENTS, "readwrite");
    transaction.objectStore(SEGMENTS).delete([sessionKey, segmentId]);
    await transactionDone(transaction);
  }

  async putChunk(chunk: StoredChunk): Promise<void> {
    const transaction = this.db.transaction([CHUNKS, CHUNK_INDEX], "readwrite");
    transaction.objectStore(CHUNKS).put({ ...chunk });
    transaction.objectStore(CHUNK_INDEX).put({
      sessionKey: chunk.sessionKey,
      segmentId: chunk.segmentId,
      seq: chunk.seq,
      size: chunk.bytes.byteLength,
    });
    await transactionDone(transaction);
  }

  async listChunks(sessionKey: string, segmentId: string): Promise<ChunkInfo[]> {
    const transaction = this.db.transaction(CHUNK_INDEX, "readonly");
    const range = IDBKeyRange.bound([sessionKey, segmentId, 0], [sessionKey, segmentId, Number.MAX_SAFE_INTEGER]);
    const rows = (await requestToPromise(transaction.objectStore(CHUNK_INDEX).getAll(range))) as Array<
      ChunkKey & { size: number }
    >;
    return rows.map((row) => ({ seq: row.seq, size: row.size })).sort((a, b) => a.seq - b.seq);
  }

  async getChunk(key: ChunkKey): Promise<StoredChunk | undefined> {
    const transaction = this.db.transaction(CHUNKS, "readonly");
    const row = await requestToPromise(
      transaction.objectStore(CHUNKS).get([key.sessionKey, key.segmentId, key.seq]),
    );
    return row as StoredChunk | undefined;
  }

  async deleteChunk(key: ChunkKey): Promise<void> {
    const transaction = this.db.transaction([CHUNKS, CHUNK_INDEX], "readwrite");
    const id = [key.sessionKey, key.segmentId, key.seq];
    transaction.objectStore(CHUNKS).delete(id);
    transaction.objectStore(CHUNK_INDEX).delete(id);
    await transactionDone(transaction);
  }
}
