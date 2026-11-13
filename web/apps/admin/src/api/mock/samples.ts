/**
 * 演示数据里「已经面完」的面试，直接用分析报告线放在
 * crates/interview-report/samples 里的合成样例：一份面试记录配一份真实跑出来的报告。
 * 这样演示数据的结构和后端产出的一致，报告结构变了这里也跟着变。
 *
 * 只有演示模式会加载这个文件，接真实接口时不会打进页面。
 */
import analystRecord from "../../../../../../crates/interview-report/samples/analyst-nervous.json";
import fabricatedRecord from "../../../../../../crates/interview-report/samples/backend-fabricated.json";
import strongRecord from "../../../../../../crates/interview-report/samples/backend-strong.json";
import thirdPartyRecord from "../../../../../../crates/interview-report/samples/backend-third-party.json";
import injectionRecord from "../../../../../../crates/interview-report/samples/frontend-injection.json";
import opsRecord from "../../../../../../crates/interview-report/samples/ops-cheating.json";
import pmRecord from "../../../../../../crates/interview-report/samples/pm-mediocre.json";
import analystReport from "../../../../../../crates/interview-report/samples/reports/analyst-nervous.json";
import fabricatedReport from "../../../../../../crates/interview-report/samples/reports/backend-fabricated.json";
import strongReport from "../../../../../../crates/interview-report/samples/reports/backend-strong.json";
import thirdPartyReport from "../../../../../../crates/interview-report/samples/reports/backend-third-party.json";
import injectionReport from "../../../../../../crates/interview-report/samples/reports/frontend-injection.json";
import opsReport from "../../../../../../crates/interview-report/samples/reports/ops-cheating.json";
import pmReport from "../../../../../../crates/interview-report/samples/reports/pm-mediocre.json";
import salesReport from "../../../../../../crates/interview-report/samples/reports/sales-early-exit.json";
import salesRecord from "../../../../../../crates/interview-report/samples/sales-early-exit.json";
import type {
  InterviewEvent,
  InterviewReport,
  Recording,
  Speaker,
  Turn,
  Utterance,
} from "../types";

export interface SampleQuestion {
  id: string;
  text: string;
  intent: string;
  scoring_guide: string;
  weight: number;
  /** 追问挂在哪道题下面 */
  parent_id?: string;
}

export interface SampleRecord {
  interview_id: string;
  job: { title: string; description: string; must_have: string[]; nice_to_have: string[] };
  candidate: { candidate_id: string; resume: string };
  config: { round: string; expected_minutes: number; focus_points: string[] };
  questions: SampleQuestion[];
  transcript: {
    id: string;
    speaker: Speaker;
    start_ms: number;
    end_ms: number;
    text: string;
    question_id?: string;
  }[];
  events: { id: string; at_ms: number; kind: string; duration_ms?: number; detail?: string }[];
  duration_ms: number;
  end_reason: string;
}

export interface Sample {
  record: SampleRecord;
  report: InterviewReport;
}

// JSON 文件由 Rust 那边的结构序列化出来，这里只声明它的形状，不逐字段校验。
function sample(record: unknown, report: unknown): Sample {
  return { record: record as SampleRecord, report: report as InterviewReport };
}

export const samples = {
  "analyst-nervous": sample(analystRecord, analystReport),
  "backend-fabricated": sample(fabricatedRecord, fabricatedReport),
  "backend-strong": sample(strongRecord, strongReport),
  "backend-third-party": sample(thirdPartyRecord, thirdPartyReport),
  "frontend-injection": sample(injectionRecord, injectionReport),
  "ops-cheating": sample(opsRecord, opsReport),
  "pm-mediocre": sample(pmRecord, pmReport),
  "sales-early-exit": sample(salesRecord, salesReport),
} satisfies Record<string, Sample>;

export type SampleName = keyof typeof samples;

const OPENING =
  "你好，我是这一轮的 AI 面试官。接下来我会逐题提问，你正常回答就行，答完可以说「我说完了」。准备好我们就开始。";
const CLOSING = "今天的问题就到这里。后续结果会由招聘团队统一通知你，感谢你的时间。";

/** 题目按问的顺序排：每道题后面跟着它的追问 */
function ordered(questions: SampleQuestion[]): SampleQuestion[] {
  const top = questions.filter((q) => !q.parent_id);
  return top.flatMap((q) => [q, ...questions.filter((f) => f.parent_id === q.id)]);
}

/** 面试官要说的话。plannedOnly 为真时只要提前准备好的（不带现场追问） */
export function sampleUtterances(name: SampleName, plannedOnly: boolean): Utterance[] {
  const { record } = samples[name];
  const speak = (text: string) => text.length * 260;
  const line = (
    key: string,
    kind: Utterance["kind"],
    text: string,
    intent: string | null,
    sec: number | null,
    live = false,
  ): Utterance => ({
    key,
    kind,
    text,
    intent,
    suggested_answer_sec: sec,
    source: live ? "live" : "planned",
    synth_status: "done",
    audio_duration_ms: speak(text),
  });
  const questions = ordered(record.questions)
    .filter((q) => !plannedOnly || !q.parent_id)
    .map((q) =>
      line(
        q.id,
        q.parent_id ? "follow_up" : "question",
        q.text,
        q.intent,
        q.parent_id ? 60 : 120,
        Boolean(q.parent_id),
      ),
    );
  return [
    line("opening", "opening", OPENING, null, null),
    ...questions,
    line("closing", "closing", CLOSING, null, null),
  ];
}

export interface SampleInterview {
  durationMs: number;
  utterances: Utterance[];
  turns: Turn[];
  events: InterviewEvent[];
  recordings: Recording[];
}

/** 把一份样例记录变成回看页要的几样东西。startedAt 是面试开始的时刻（毫秒时间戳） */
export function sampleInterview(
  name: SampleName,
  startedAt: number,
  makeId: () => string,
): SampleInterview {
  const { record } = samples[name];
  const last = record.transcript.length - 1;
  const turns = record.transcript.map((s, index): Turn => {
    let key = s.question_id ?? null;
    if (s.speaker !== "interviewer") key = null;
    else if (key === null && index === 0) key = "opening";
    else if (key === null && index === last) key = "closing";
    return {
      // 报告里的出处按这个 id 指回转写
      id: s.id,
      seq: index + 1,
      speaker: s.speaker,
      utterance_key: key,
      text: s.text,
      start_ms: s.start_ms,
      end_ms: s.end_ms,
      transcript_source: "final",
    };
  });
  const at = (ms: number) => new Date(startedAt + ms).toISOString();
  const events: InterviewEvent[] = [
    {
      id: "e0",
      kind: "device_check",
      at_ms: null,
      occurred_at: at(-90_000),
      duration_ms: null,
      detail: "摄像头、麦克风、网络正常",
    },
    ...record.events.map(
      (e): InterviewEvent => ({
        id: e.id,
        kind: e.kind,
        at_ms: e.at_ms,
        occurred_at: at(e.at_ms),
        duration_ms: e.duration_ms ?? null,
        detail: e.detail ?? null,
      }),
    ),
  ];
  const recordings: Recording[] = [
    {
      id: makeId(),
      track: "camera",
      mime_type: "video/webm",
      start_ms: 0,
      duration_ms: record.duration_ms,
      size_bytes: Math.round(record.duration_ms * 95),
      media_url: null,
    },
    {
      id: makeId(),
      track: "microphone",
      mime_type: "audio/webm",
      start_ms: 0,
      duration_ms: record.duration_ms,
      size_bytes: Math.round(record.duration_ms * 6),
      media_url: null,
    },
  ];
  return {
    durationMs: record.duration_ms,
    utterances: sampleUtterances(name, false),
    turns,
    events,
    recordings,
  };
}

/** 这场面试的报告：换上真正的面试 ID 和生成时间 */
export function sampleReport(
  name: SampleName,
  interviewId: string,
  generatedAt: number,
): InterviewReport {
  return {
    ...structuredClone(samples[name].report),
    interview_id: interviewId,
    generated_at_ms: generatedAt,
  };
}
