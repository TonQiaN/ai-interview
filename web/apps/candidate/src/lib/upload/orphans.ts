/**
 * 页面刷新、崩溃或被系统回收之后，上一次页面生命里没来得及正常结束的录制段会留在本地：
 * 分片还在，但段没有登记「一共多少片」，上传器不知道什么时候该让服务端合并。
 * 这里把这些段补上分片总数，交还给上传器。
 */
import type { ChunkStore } from "./chunk-store";
import type { UploadApi } from "./uploader";

/**
 * @returns 还没能收尾的段数（本地没有分片、又问不到服务端时会留到下次）。
 */
export async function closeOrphanSegments(
  store: ChunkStore,
  sessionKey: string,
  api: Pick<UploadApi, "receivedChunks">,
): Promise<number> {
  let unresolved = 0;
  for (const segment of await store.listSegments(sessionKey)) {
    if (segment.totalChunks !== null) continue;

    const local = await store.listChunks(sessionKey, segment.segmentId);
    let total: number;
    if (local.length > 0) {
      // 分片按序号上传，本地剩下的一定是序号最大的那些，不用问服务端
      total = Math.max(...local.map((chunk) => chunk.seq)) + 1;
    } else {
      try {
        const received = await api.receivedChunks({
          track: segment.track,
          segmentId: segment.segmentId,
        });
        total = received.length === 0 ? 0 : Math.max(...received) + 1;
      } catch {
        unresolved += 1;
        continue;
      }
    }

    if (total === 0) {
      // 一个分片都没录到（刚开始录就刷新了），直接丢掉
      await store.deleteSegment(sessionKey, segment.segmentId);
    } else {
      await store.putSegment({ ...segment, totalChunks: total });
    }
  }
  return unresolved;
}
