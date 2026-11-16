// 题目出好以后 HR 自己加一道题：不调模型，直接放进计划里，后端只给新的这一句合成语音。
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
    scoring: { strong_signals: [], weak_signals: [], red_flags: [] },
    follow_ups: [],
  };
  const firstReserve = plan.questions.findIndex((q) => q.reserve);
  const at = firstReserve === -1 ? plan.questions.length : firstReserve;
  return {
    ...plan,
    questions: [...plan.questions.slice(0, at), question, ...plan.questions.slice(at)],
  };
}

/** 预计回答多久可以选的几档，不超过设置里的单题最长时间；一档都不剩时就用那个上限 */
export function answerTimeChoices(maxSec: number): number[] {
  const choices = [60, 120, 180, 300].filter((sec) => sec <= maxSec);
  return choices.length > 0 ? choices : [maxSec];
}
