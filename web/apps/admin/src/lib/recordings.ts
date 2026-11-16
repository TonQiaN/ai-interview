/** 回看页上要告诉 HR 的录制情况：哪一段不完整、哪一段没保存下来、哪一段还在整理。 */
import type { Recording } from "../api/types";
import { clock } from "./format";

export const trackLabel: Record<Recording["track"], string> = {
  camera: "录像",
  microphone: "录音",
  screen: "屏幕",
};

export interface RecordingNote {
  key: string;
  /** warning：这一段有问题；info：还在上传和整理 */
  tone: "warning" | "info";
  text: string;
}

interface Named {
  recording: Recording;
  /** 「录像」或者「录像第 2 段」 */
  name: string;
  /** 这一路不止一段时，说明是从哪里开始的那一段 */
  from: string;
}

/** 同一路的各段，按时间排 */
function sameTrack(recordings: Recording[], recording: Recording): Recording[] {
  return recordings
    .filter((r) => r.track === recording.track)
    .sort((a, b) => a.start_ms - b.start_ms);
}

/** 一段录制怎么称呼：这一路只有一段叫「录像」，有好几段（断线重连过）叫「录像第 2 段」 */
export function recordingName(recordings: Recording[], recording: Recording): string {
  const same = sameTrack(recordings, recording);
  const nth = same.length > 1 ? `第 ${same.indexOf(recording) + 1} 段` : "";
  return `${trackLabel[recording.track]}${nth}`;
}

function named(recordings: Recording[]): Named[] {
  return [...recordings]
    .sort((a, b) => a.start_ms - b.start_ms)
    .map((recording) => ({
      recording,
      name: recordingName(recordings, recording),
      from:
        sameTrack(recordings, recording).length > 1
          ? `（从 ${clock(recording.start_ms)} 开始）`
          : "",
    }));
}

/** 去掉结尾的句号，后面好接话 */
const bare = (text: string) => text.trim().replace(/[。.]$/, "");

/**
 * 录制里值得说明的情况，按时间排。
 *
 * 能播但不完整的（候选人没传完，后端把收到的部分合并出来）和没保存下来的，原因后端已经写成给 HR
 * 看的话，这里原样用。录像和录音同一段出了同样的问题时并成一句。
 */
export function recordingNotes(recordings: Recording[]): RecordingNote[] {
  const all = named(recordings);
  const notes: RecordingNote[] = [];
  const groups = new Map<string, { names: string[]; from: string; text: string }>();

  for (const { recording: r, name, from } of all) {
    const failed = r.status === "failed";
    const reason = bare(r.error ?? "");
    if (!failed && !(r.status === "ready" && reason)) continue;
    const text = failed ? `${reason || "这一段没有保存下来"}。这一段只能看逐字稿。` : `${reason}。`;
    const key = `${r.start_ms}|${text}`;
    const group = groups.get(key);
    if (group) group.names.push(name);
    else groups.set(key, { names: [name], from, text });
  }
  for (const [key, group] of groups)
    notes.push({
      key,
      tone: "warning",
      text: `${group.names.join("和")}${group.from}：${group.text}`,
    });

  const pending = all.filter(
    ({ recording: r }) => r.status === "uploading" || r.status === "assembling",
  );
  if (pending.length > 0)
    notes.push({
      key: "pending",
      tone: "info",
      text: `${pending.map((p) => p.name).join("、")}还在上传和整理，过一会儿再来看。候选人那边没传完的话，面试结束后要等半小时左右。`,
    });
  return notes;
}

/**
 * 候选人的上行网速装不下平时的码率时，面试页会把录像的画面降下来（`interview_started` 里
 * `video_reduced` 为 true）。回看时画面比平时糊，要让 HR 知道这不是摄像头的问题。没降过就是 null。
 */
export function reducedVideoNote(
  events: { kind: string; at_ms?: number | null; payload: Record<string, unknown> }[],
): string | null {
  const started = events.filter((e) => e.kind === "interview_started");
  const reduced = started.filter((e) => e.payload?.video_reduced === true);
  if (reduced.length === 0) return null;
  const which =
    reduced.length === started.length
      ? "这场面试"
      : `从 ${reduced.map((e) => clock(e.at_ms ?? 0)).join("、")} 开始的那一段`;
  return `候选人的上行网速慢，面试页把${which}录像的画面降了清晰度（声音没有降）。画面比平时糊，不是候选人的摄像头有问题。`;
}
