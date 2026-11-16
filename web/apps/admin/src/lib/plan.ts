// 题目出好以后 HR 自己调：加一道题、给一道题加追问、调先后、改哪些题时间紧也要问、主问题和备用题互换。都不调模型，
// 改完整份计划存回去，后端只给文字变了的和新增的句子合成语音。
import type { InterviewPlan, PlannedQuestion, QuestionCategory } from "../api/types";

/** 页面上填的那几样 */
export interface QuestionDraft {
  /** 面试官要说的话 */
  text: string;
  /** 想考察什么。判断回答和写报告的模型都会看，不念给候选人 */
  intent: string;
  /** 考察哪个维度，是这场面试的维度里的一个名字 */
  competency: string;
  category: QuestionCategory;
  expectedAnswerSec: number;
  /** 答得好会提到什么，一行一条，可以不填。判断回答和写报告的模型会对照 */
  strongSignals: string;
}

/** 给新题起一个这份计划里没用过的名字。出题时用的是 q1、q2……，接着往后排 */
export function nextQuestionKey(plan: InterviewPlan): string {
  const used = new Set(plan.questions.map((q) => q.key));
  let number =
    plan.questions.reduce((max, q) => {
      const found = /^q(\d+)$/.exec(q.key);
      return found ? Math.max(max, Number(found[1])) : max;
    }, 0) + 1;
  while (used.has(`q${number}`)) number += 1;
  return `q${number}`;
}

/**
 * 把 HR 写的题放进计划：排在主问题的最后、备用题的前面（后端要求备用题都在主问题后面）。
 * 标成必问题：HR 特意加的，时间紧也不该被跳过（引擎只在超出预期时长 125% 以后才不问必问题）。
 */
export function addQuestion(plan: InterviewPlan, draft: QuestionDraft): InterviewPlan {
  const question: PlannedQuestion = {
    key: nextQuestionKey(plan),
    text: draft.text.trim(),
    intent: draft.intent.trim(),
    category: draft.category,
    competency: draft.competency,
    origin: "must_ask",
    required: true,
    reserve: false,
    expected_answer_sec: draft.expectedAnswerSec,
    scoring: { strong_signals: lines(draft.strongSignals), weak_signals: [], red_flags: [] },
    follow_ups: [],
  };
  const firstReserve = plan.questions.findIndex((q) => q.reserve);
  const at = firstReserve === -1 ? plan.questions.length : firstReserve;
  return {
    ...plan,
    questions: [...plan.questions.slice(0, at), question, ...plan.questions.slice(at)],
  };
}

/** 加一句追问时页面上填的 */
export interface FollowUpDraft {
  /** 面试官追问时要说的话 */
  text: string;
  /** 什么情况下问这一句。判断回答的模型会看，不念给候选人；可以不填 */
  askWhen: string;
}

/** 没写什么情况下问时用的那一句 */
export const DEFAULT_ASK_WHEN = "候选人的回答里没有讲到这一点的时候";

/** 这道题还能不能再加追问：设置里关了追问、或者已经到了每题的上限就不能（后端会拒绝超过上限的） */
export function canAddFollowUp(
  question: PlannedQuestion,
  followUps: { enabled: boolean; max_per_question: number },
): boolean {
  return followUps.enabled && (question.follow_ups ?? []).length < followUps.max_per_question;
}

/** 给新追问起名字：后端要求以所属题目的 key 开头，出题时用的是 q1.f1、q1.f2……，接着往后排 */
export function nextFollowUpKey(plan: InterviewPlan, questionKey: string): string {
  const used = new Set(
    plan.questions.flatMap((q) => [q.key, ...(q.follow_ups ?? []).map((f) => f.key)]),
  );
  let number = 1;
  while (used.has(`${questionKey}.f${number}`)) number += 1;
  return `${questionKey}.f${number}`;
}

/**
 * 给一道题加一句 HR 写的追问，排在这道题已有追问的后面。找不到这道题时原样返回。
 * 逐题问答里，候选人的回答符合「什么情况下问」时面试官照原话追问；
 * 实时对话里面试官现场组织追问，这一句是给它的方向。
 */
export function addFollowUp(
  plan: InterviewPlan,
  questionKey: string,
  draft: FollowUpDraft,
): InterviewPlan {
  if (!plan.questions.some((q) => q.key === questionKey)) return plan;
  const followUp = {
    key: nextFollowUpKey(plan, questionKey),
    text: draft.text.trim(),
    ask_when: draft.askWhen.trim() || DEFAULT_ASK_WHEN,
  };
  return {
    ...plan,
    questions: plan.questions.map((q) =>
      q.key === questionKey ? { ...q, follow_ups: [...(q.follow_ups ?? []), followUp] } : q,
    ),
  };
}

/** 一行一条，空行和每行前后的空白、列表符号去掉 */
function lines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.replace(/^\s*(?:[-*•·]|\d+[.、)])\s*/, "").trim())
    .filter((line) => line !== "");
}

/**
 * 主问题往前（-1）或往后（1）挪一位以后排在第几个；挪不了就是 undefined。
 * 只在主问题之间挪：备用题要排在所有主问题后面（后端会检查），不参与。
 */
function moveTarget(plan: InterviewPlan, key: string, direction: -1 | 1): number | undefined {
  const from = plan.questions.findIndex((q) => q.key === key);
  const to = from + direction;
  const moving = plan.questions[from];
  const other = plan.questions[to];
  if (!moving || !other || moving.reserve || other.reserve) return undefined;
  return to;
}

/** 这道主问题还能不能往这个方向挪 */
export function canMoveQuestion(plan: InterviewPlan, key: string, direction: -1 | 1): boolean {
  return moveTarget(plan, key, direction) !== undefined;
}

/**
 * 把一道主问题往前或往后挪一位，和旁边那道换个位置。挪不了时原样返回。
 * 面试官要说的话没有变，后端不会重新合成语音。时间不够时没标必问的题会被跳过，
 * 越靠后越容易轮不上，所以先后是有用的。
 */
export function moveQuestion(plan: InterviewPlan, key: string, direction: -1 | 1): InterviewPlan {
  const to = moveTarget(plan, key, direction);
  if (to === undefined) return plan;
  const from = to - direction;
  const questions = [...plan.questions];
  const [moving] = questions.splice(from, 1);
  if (!moving) return plan;
  questions.splice(to, 0, moving);
  return { ...plan, questions };
}

/**
 * 把一道主问题标成「时间紧也要问」或者「时间紧可以跳过」。
 * 备用题不能标成必问（后端会拒绝），原样返回。
 */
export function setRequired(plan: InterviewPlan, key: string, required: boolean): InterviewPlan {
  return {
    ...plan,
    questions: plan.questions.map((q) => (q.key === key && !q.reserve ? { ...q, required } : q)),
  };
}

/** 这道题能不能换到另一边：主问题只剩一道时不能再改成备用题（后端要求至少一道主问题） */
export function canSetReserve(plan: InterviewPlan, key: string, reserve: boolean): boolean {
  const question = plan.questions.find((q) => q.key === key);
  if (!question || question.reserve === reserve) return false;
  return !reserve || plan.questions.filter((q) => !q.reserve).length > 1;
}

/**
 * 把一道主问题改成备用题（时间有富余才问），或者把一道备用题改成主问题。换不了时原样返回。
 * 备用题要排在所有主问题后面（后端会检查）：改成备用的排到备用题的最前面，改成主问题的排到主问题的最后。
 * 备用题不能是必问的，改成备用时一起去掉；改成主问题以后要不要「时间紧也要问」由 HR 再定。
 * 面试官要说的话没有变，后端不会重新合成语音。
 */
export function setReserve(plan: InterviewPlan, key: string, reserve: boolean): InterviewPlan {
  if (!canSetReserve(plan, key, reserve)) return plan;
  const moving = plan.questions.find((q) => q.key === key);
  if (!moving) return plan;
  const rest = plan.questions.filter((q) => q.key !== key);
  const firstReserve = rest.findIndex((q) => q.reserve);
  const at = firstReserve === -1 ? rest.length : firstReserve;
  return {
    ...plan,
    questions: [...rest.slice(0, at), { ...moving, reserve, required: false }, ...rest.slice(at)],
  };
}

/** 预计回答多久可以选的几档，不超过设置里的单题最长时间；一档都不剩时就用那个上限 */
export function answerTimeChoices(maxSec: number): number[] {
  const choices = [60, 120, 180, 300].filter((sec) => sec <= maxSec);
  return choices.length > 0 ? choices : [maxSec];
}
