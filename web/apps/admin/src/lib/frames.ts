import { isMock } from "../api/http";
import type { FrameEvent, InterviewEvent } from "../api/types";

/** 录像里存了图的一帧：在面试时间轴上的位置，和取图的地址 */
export interface Frame {
  at_ms: number;
  url: string;
}

/** 时间轴上的一条：过程事件，或者出报告前检查录像画面得到的结果（带着存了图的帧） */
export type TimelineEvent = InterviewEvent & { frames?: Frame[] };

/** 演示数据没有真的录像，放一张示意图 */
const demoFrame = `data:image/svg+xml;utf8,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><rect width="160" height="90" fill="#4a4338"/><circle cx="80" cy="38" r="15" fill="#b9a98a"/><path d="M48 90c2-22 14-34 32-34s30 12 32 34z" fill="#b9a98a"/></svg>',
)}`;

/** 一帧画面的地址。要登录才取得到，和页面同源，可以直接给 img 用 */
export function frameUrl(interviewId: string, atMs: number): string {
  return isMock ? demoFrame : `/api/v1/interviews/${interviewId}/frames/${atMs}`;
}

const framesOf = (interviewId: string, event: FrameEvent): Frame[] =>
  event.frame_at_ms.map((at_ms) => ({ at_ms, url: frameUrl(interviewId, at_ms) }));

/** 报告里的证据用 event_id 指向画面检查的结果，按它找到存了图的帧 */
export function framesByEvent(interviewId: string, items: FrameEvent[]): Map<string, Frame[]> {
  const out = new Map<string, Frame[]>();
  for (const item of items) {
    const frames = framesOf(interviewId, item);
    if (frames.length > 0) out.set(item.id, frames);
  }
  return out;
}

/**
 * 把画面检查的结果按时间排进过程事件里。两边用的是同一条时间轴（at_ms）。
 * 没有时间的那条（一共抽了多少帧）是面试结束以后才有的，放在最后。
 */
export function withFrameEvents(
  interviewId: string,
  events: InterviewEvent[],
  frameEvents: FrameEvent[],
): TimelineEvent[] {
  const asTimeline = (item: FrameEvent): TimelineEvent => ({
    id: item.id,
    kind: item.kind,
    at_ms: item.at_ms ?? null,
    occurred_at: "",
    source: "frames",
    payload: item.payload,
    frames: framesOf(interviewId, item),
  });
  const timed = frameEvents
    .filter((f) => typeof f.at_ms === "number")
    .sort((a, b) => (a.at_ms ?? 0) - (b.at_ms ?? 0));
  const untimed = frameEvents.filter((f) => typeof f.at_ms !== "number");

  const out: TimelineEvent[] = [];
  let next = 0;
  /** 最后一条有时间的事件后面的位置。比所有过程事件都晚的画面事件放这里，不能排到「结束后」的事件后面 */
  let afterTimed = -1;
  for (const event of events) {
    if (typeof event.at_ms === "number") {
      while (next < timed.length && (timed[next]?.at_ms ?? 0) < event.at_ms) {
        out.push(asTimeline(timed[next] as FrameEvent));
        next += 1;
      }
      out.push(event);
      afterTimed = out.length;
    } else {
      out.push(event);
    }
  }
  const rest = timed.slice(next).map(asTimeline);
  if (afterTimed < 0) out.push(...rest);
  else out.splice(afterTimed, 0, ...rest);
  out.push(...untimed.map(asTimeline));
  return out;
}

/** 这一条是不是画面检查的结果 */
export const isFrameEvent = (event: { source: string }): boolean => event.source === "frames";
