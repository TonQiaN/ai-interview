// 改了一场面试的设置之后，已经出好的题目还能不能用，以及怎么跟 HR 说。
// 规则是后端定的（crates/interview-engine/README.md「改了配置，题目还能不能用」），
// 真实后端在返回里用 replan_reasons 告诉我们；这里的 replanReasons 只给演示数据用。
import type { InterviewConfig } from "../api/types";
import { estimateBudget } from "./budget";

/** 后端给的字段路径在设置表单上叫什么 */
const fieldLabel: Record<string, string> = {
  resume_id: "简历",
  target_duration_minutes: "预期时长",
  stage: "这是第几轮",
  process_note: "这场面试在流程里的位置",
  language: "面试语言",
  seniority: "岗位的资历",
  "interviewer.name": "面试官的称呼",
  "interviewer.style": "说话风格",
  company_intro: "公司和团队介绍",
  focus_instructions: "重点考察",
  must_ask_questions: "必须问的题",
  avoid_topics: "不要问的话题",
  competencies: "考察维度",
  screening_checks: "要确认的基本条件",
  prior_notes: "之前已经了解到的情况",
  verify_resume: "核实简历",
  candidate_questions: "候选人提问环节",
  question_count: "题目数量",
  "answer.silence_sec": "停顿多久算答完",
  "follow_ups.enabled": "追问",
  "follow_ups.max_per_question": "追问次数",
};

/** 把导致重新出题的字段写成一句话里能用的名字，最多列三个。 */
export function replanReasonText(reasons: string[]): string {
  const names: string[] = [];
  for (const reason of reasons) {
    const name = fieldLabel[reason] ?? "其他设置";
    if (!names.includes(name)) names.push(name);
  }
  if (names.length <= 3) return names.join("、");
  return `${names.slice(0, 3).join("、")}等 ${names.length} 项`;
}

/** 保存设置之后提示 HR 的那句话。 */
export function updateOutcome({
  hadPlan,
  voiceChanged,
  reasons,
}: {
  /** 保存之前题目是不是已经出好了 */
  hadPlan: boolean;
  voiceChanged: boolean;
  /** 后端返回的 replan_reasons */
  reasons: string[];
}): string {
  if (reasons.length > 0) return `改了${replanReasonText(reasons)}，题目和语音要重新准备`;
  if (!hadPlan) return "设置已保存";
  if (voiceChanged) return "换了声音，题目不变，语音要重新合成";
  return "设置已保存，题目和语音都还能用";
}

/** 写进了题目和面试官的话里的设置：改了就要重新出题 */
const inPlan: [string, (c: InterviewConfig) => unknown][] = [
  ["target_duration_minutes", (c) => c.target_duration_minutes],
  ["stage", (c) => c.stage],
  ["process_note", (c) => c.process_note],
  ["language", (c) => c.language],
  ["seniority", (c) => c.seniority],
  ["interviewer.name", (c) => c.interviewer.name],
  ["interviewer.style", (c) => c.interviewer.style],
  ["company_intro", (c) => c.company_intro],
  ["focus_instructions", (c) => c.focus_instructions],
  ["must_ask_questions", (c) => c.must_ask_questions],
  ["avoid_topics", (c) => c.avoid_topics],
  ["competencies", (c) => c.competencies],
  ["screening_checks", (c) => c.screening_checks],
  ["prior_notes", (c) => c.prior_notes],
  ["verify_resume", (c) => c.verify_resume],
  ["candidate_questions", (c) => c.candidate_questions],
];

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** 配置从 old 改成 next，哪些字段让已经出好的题目不能用了。和后端的规则一样，只给演示数据用。 */
export function replanReasons(old: InterviewConfig, next: InterviewConfig): string[] {
  const reasons = inPlan.filter(([, read]) => !same(read(old), read(next))).map(([path]) => path);

  // 题目数量、静音秒数、追问只通过题数起作用：只换这三项，看题数变不变
  const probe: InterviewConfig = {
    ...old,
    question_count: next.question_count,
    answer: next.answer,
    follow_ups: next.follow_ups,
  };
  const [before, after] = [estimateBudget(old), estimateBudget(probe)];
  const countsChanged =
    before.question_count !== after.question_count ||
    before.reserve_question_count !== after.reserve_question_count;
  const allowed = (f: InterviewConfig["follow_ups"]) => (f.enabled ? f.max_per_question : 0);
  // 追问调少了还能用，调多了要重新出：多出来的追问要提前写好
  const moreFollowUps = allowed(next.follow_ups) > allowed(old.follow_ups);

  if (countsChanged && !same(old.question_count, next.question_count))
    reasons.push("question_count");
  if (countsChanged && old.answer.silence_sec !== next.answer.silence_sec)
    reasons.push("answer.silence_sec");
  if ((countsChanged || moreFollowUps) && old.follow_ups.enabled !== next.follow_ups.enabled)
    reasons.push("follow_ups.enabled");
  if (
    (countsChanged || moreFollowUps) &&
    old.follow_ups.max_per_question !== next.follow_ups.max_per_question
  )
    reasons.push("follow_ups.max_per_question");
  return reasons;
}
