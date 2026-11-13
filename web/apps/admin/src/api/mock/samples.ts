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
  InterviewConfig,
  InterviewEvent,
  InterviewPlan,
  InterviewReport,
  PlannedQuestion,
  QuestionCategory,
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
const CLOSING_EARLY = "好的，那今天就先到这里。后续安排会由招聘团队和你联系，感谢你的时间。";
const NUDGE = "你还在吗？如果需要再想一想，可以告诉我。";
const REPEAT = "没关系，我再说一遍。";
const ACKS = ["好的，谢谢。", "明白了，我们看下一题。"];
const CANDIDATE_QUESTIONS = "我的问题问完了。你有什么想了解的吗？我会记下来转给招聘团队。";

/** 按题目的问法猜一个分类，只用来让演示数据看起来像样 */
function categoryOf(q: SampleQuestion, index: number): QuestionCategory {
  if (/假设|如果/.test(q.text)) return "situational";
  if (/为什么.*(离开|选择|想)|期望|打算/.test(q.text)) return "motivation";
  if (/推理|估算|多少/.test(q.text)) return "logic";
  if (index === 0 || /简历|项目|经历/.test(q.text)) return "resume_verification";
  return "professional";
}

const COMPETENCY: Record<QuestionCategory, string> = {
  warmup: "热身",
  resume_verification: "经历真实性",
  professional: "专业能力",
  behavioral: "行为表现",
  situational: "分析和应对",
  logic: "逻辑推理",
  motivation: "求职动机",
  screening: "基本条件",
  other: "其他",
};

/** 样例里的评分标准是一句话，用分号隔开「好的表现」和「差的表现」 */
function scoring(guide: string): PlannedQuestion["scoring"] {
  const [strong = guide, ...weak] = guide.split("；");
  return { strong_signals: [strong], weak_signals: weak, red_flags: [] };
}

/** 一份样例对应的题目计划，结构和 AI 引擎出的一样 */
export function samplePlan(name: SampleName, config: InterviewConfig): InterviewPlan {
  const { record } = samples[name];
  const top = record.questions.filter((q) => !q.parent_id);
  const focus = new Set(record.config.focus_points);
  const questions = top.map((q, index): PlannedQuestion => {
    const category = categoryOf(q, index);
    return {
      key: q.id,
      text: q.text,
      intent: q.intent,
      category,
      competency: COMPETENCY[category],
      expected_answer_sec: category === "resume_verification" ? 150 : 100,
      origin: focus.size > 0 && index === 2 ? "focus" : "generated",
      required: index < top.length - 1,
      reserve: false,
      follow_ups: config.follow_ups.enabled
        ? record.questions
            .filter((f) => f.parent_id === q.id)
            .slice(0, config.follow_ups.max_per_question)
            .map((f) => ({
              key: f.id,
              text: f.text,
              ask_when: "回答比较笼统、没有讲到自己具体怎么做的时候",
            }))
        : [],
      scoring: scoring(q.scoring_guide),
    };
  });
  const names = [...new Set(questions.map((q) => q.competency))];
  return {
    schema_version: 1,
    expected_duration_sec: config.target_duration_minutes * 60,
    brief: {
      role_summary: record.job.description.split("要求：")[0] ?? record.job.description,
      candidate_summary: "简历和岗位方向对得上，几段经历里自己负责的部分需要在面试里核实。",
      competencies: names.map((n) => ({ name: n, description: "", weight: 3 })),
      must_haves: record.job.must_have,
      nice_to_haves: record.job.nice_to_have,
      matches: record.job.must_have.slice(0, 2),
      gaps: record.job.nice_to_have.slice(0, 1).map((item) => `简历里没有提到：${item}`),
      verify_points: record.config.focus_points,
    },
    phrases: {
      opening: OPENING,
      closing: CLOSING,
      closing_early: CLOSING_EARLY,
      nudge: NUDGE,
      repeat: REPEAT,
      acks: ACKS,
      candidate_questions: config.candidate_questions === "collect" ? CANDIDATE_QUESTIONS : null,
    },
    questions,
    generation: {
      model: "deepseek-flash",
      llm_calls: 3,
      review_notes: [],
      system_fingerprint: null,
      usage: {
        prompt_tokens: 5200,
        completion_tokens: 2100,
        reasoning_tokens: 1800,
        cache_hit_tokens: 0,
        cache_miss_tokens: 5200,
      },
    },
  };
}

/** 一份计划里面试官要说的每一句话，顺序和键跟后端一致 */
export function planUtterances(plan: InterviewPlan, synthesized: boolean): Utterance[] {
  const list: Utterance[] = [];
  const push = (key: string, kind: Utterance["kind"], text: string) => {
    list.push({
      key,
      kind,
      seq: list.length + 1,
      source: "planned",
      text,
      synth_status: synthesized ? "done" : "pending",
      synth_error: null,
      has_audio: synthesized,
      audio_duration_ms: synthesized ? text.length * 260 : null,
      tts_engine: synthesized ? "index2" : null,
    });
  };
  push("opening", "opening", plan.phrases.opening);
  for (const q of plan.questions) {
    push(q.key, "question", q.text);
    for (const f of q.follow_ups ?? []) push(f.key, "follow_up", f.text);
  }
  plan.phrases.acks.forEach((ack, index) => {
    push(`ack.${index + 1}`, "transition", ack);
  });
  push("nudge", "other", plan.phrases.nudge);
  push("repeat", "other", plan.phrases.repeat);
  if (plan.phrases.candidate_questions) {
    push("candidate_questions", "question", plan.phrases.candidate_questions);
  }
  push("closing", "closing", plan.phrases.closing);
  push("closing_early", "closing", plan.phrases.closing_early);
  return list;
}

export interface SampleInterview {
  durationMs: number;
  turns: Turn[];
  events: InterviewEvent[];
  recordings: Recording[];
}

/** 成对的事件：离开的那条在前，回来的那条带离开了多久 */
const RETURN_KIND: Record<string, string> = {
  visibility_hidden: "visibility_visible",
  window_blur: "window_focus",
  network_offline: "network_online",
};

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
      words: [],
      confidence: s.speaker === "candidate" ? 0.93 : null,
      transcript_source: "final",
      end_reason: s.speaker === "candidate" ? "silence" : null,
    };
  });
  const at = (ms: number) => new Date(startedAt + ms).toISOString();
  const event = (
    id: string,
    kind: string,
    atMs: number | null,
    occurredAt: string,
    payload: Record<string, unknown> = {},
    source = "client",
  ): InterviewEvent => ({ id, kind, at_ms: atMs, occurred_at: occurredAt, source, payload });
  const events: InterviewEvent[] = [
    event("e0a", "page_opened", null, at(-150_000), {
      signals: ["visibility", "fullscreen", "device_change", "network", "clipboard"],
      in_app_browser: null,
    }),
    event("e0b", "device_check", null, at(-90_000), { result: "passed" }),
    event("e0c", "interview_started", 0, at(0), {}, "server"),
  ];
  for (const e of record.events) {
    const back = RETURN_KIND[e.kind];
    if (back) {
      // 报告里的出处指的是离开的那一条，id 保持不变
      events.push(event(e.id, e.kind, e.at_ms, at(e.at_ms)));
      const gone = e.duration_ms ?? 0;
      events.push(
        event(`${e.id}b`, back, e.at_ms + gone, at(e.at_ms + gone), { duration_ms: gone }),
      );
    } else if (e.kind === "paste" || e.kind === "copy") {
      events.push(event(e.id, e.kind, e.at_ms, at(e.at_ms), { chars: 180 }));
    } else if (e.kind === "face_count_changed") {
      const payload = { count: Number(e.detail ?? 1) };
      events.push(event(e.id, e.kind, e.at_ms, at(e.at_ms), payload, "analysis"));
    } else {
      events.push(event(e.id, e.kind, e.at_ms, at(e.at_ms), e.detail ? { detail: e.detail } : {}));
    }
  }
  events.push(
    event("e9", "interview_finished", record.duration_ms, at(record.duration_ms), {}, "server"),
  );
  events.sort((a, b) => a.occurred_at.localeCompare(b.occurred_at));

  // 断过网的那一场录像分成两段，回看时要接起来放
  const cut = record.events.find((e) => e.kind === "network_offline");
  const spans = cut
    ? [
        { start: 0, end: cut.at_ms },
        { start: cut.at_ms + (cut.duration_ms ?? 0), end: record.duration_ms },
      ]
    : [{ start: 0, end: record.duration_ms }];
  const recordings = (["camera", "microphone"] as const).flatMap((track) =>
    spans.map((span, segment): Recording => {
      const duration = span.end - span.start;
      return {
        id: makeId(),
        track,
        segment,
        content_type: track === "camera" ? "video/webm" : "audio/webm",
        status: "ready",
        size_bytes: Math.round(duration * (track === "camera" ? 95 : 6)),
        start_ms: span.start,
        duration_ms: duration,
        error: null,
        // 演示数据里没有真的录像文件
        media_url: null,
        created_at: at(span.start),
      };
    }),
  );
  return { durationMs: record.duration_ms, turns, events, recordings };
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
