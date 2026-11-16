/**
 * 清掉留在这台设备上、已经传不上去的录像分片。
 *
 * 分片里是候选人的脸和声音。平时传上去一片删一片，但有两种情况会剩下来：
 * 链接失效了（过期、被撤销、面试被取消或删除），服务端不再收；
 * 候选人关掉页面再也没回来。设备可能是借来的或者公用的，这些录像不该一直留在上面。
 */
import type { ChunkStore } from "./chunk-store";

/**
 * 别的面试留下的录像放多久算过期。服务端默认面试结束 24 小时后就不再收录像，
 * 这里放宽到 7 天，给配置得更长的部署留余地。
 */
export const STALE_AFTER_MS = 7 * 24 * 60 * 60 * 1000;

/** 删掉一场面试留在本机的所有录像。返回删了几段。 */
export async function discardSession(store: ChunkStore, sessionKey: string): Promise<number> {
  const segments = await store.listSegments(sessionKey);
  for (const segment of segments) await store.discardSegment(sessionKey, segment.segmentId);
  return segments.length;
}

export interface DiscardStaleOptions {
  /** 当前这场面试的本地键：它的录像不管多旧都不动，能不能传由服务端说了算。 */
  keep: string;
  /** 现在几点（毫秒时间戳）。 */
  now: number;
  maxAgeMs?: number;
}

/** 删掉别的面试留下的、放了太久的录像。返回删了几段。 */
export async function discardStale(
  store: ChunkStore,
  { keep, now, maxAgeMs = STALE_AFTER_MS }: DiscardStaleOptions,
): Promise<number> {
  let discarded = 0;
  for (const segment of await store.listAllSegments()) {
    if (segment.sessionKey === keep) continue;
    if (now - (segment.createdAt ?? 0) < maxAgeMs) continue;
    await store.discardSegment(segment.sessionKey, segment.segmentId);
    discarded += 1;
  }
  return discarded;
}
