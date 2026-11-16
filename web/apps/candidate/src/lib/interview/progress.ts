import type { Progress, UtteranceKind } from "../api/room-types";

/**
 * 页面上要不要显示「第 N 题 / 共 M 题」，显示的话是哪一题。
 *
 * 说结束语的时候不显示：这时候服务端给的题号是最后一题，而面试可能是提前结束的
 * （连着几题没答、答非所问、时间到了），候选人只被问到第三题，
 * 却看到「第 7 题 / 共 7 题」，会以为漏了几题。
 *
 * @returns 要显示的进度；不显示时是 null。
 */
export function questionProgressOf(
  utterance: { kind: UtteranceKind } | null,
  progress: Progress | null,
): Progress | null {
  if (progress === null || progress.index <= 0) return null;
  if (utterance?.kind === "closing") return null;
  return progress;
}
