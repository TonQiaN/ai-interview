import type { ConfigLimits, InterviewConfig, InterviewReport } from "../api/types";
import { stageLabelOf } from "./format";

/**
 * 按上一场的报告，把上一场的设置改成下一轮（深入面试）的。
 *
 * 后端出题时不会自己去看这位候选人以前的面试，上一轮发现了什么要靠设置带过去：
 * 报告建议的考察方向进「重点考察」，建议的问题进「必须问的题」，结论记进「之前已经了解到的情况」。
 * 上一场的必问题已经问过、基本条件已经确认过，不再带。
 *
 * @param when 上一场是什么时候安排的，写进沟通记录里
 */
export function nextRoundConfig(
  source: InterviewConfig,
  report: InterviewReport,
  limits: ConfigLimits,
  when: string,
): InterviewConfig {
  const clip = (text: string, max: number) => {
    const trimmed = text.trim();
    return trimmed.length > max ? `${trimmed.slice(0, max - 1)}…` : trimmed;
  };
  const list = (items: string[], max = limits.max_list_items) =>
    [...new Set(items.map((item) => clip(item, limits.max_item_chars)).filter(Boolean))].slice(
      0,
      max,
    );
  const verdict = [report.recommendation.headline, report.recommendation.overview]
    .map((part) => part.trim())
    .filter(Boolean)
    .join(" ");
  const note = verdict
    ? `${when}安排的 AI 面试（${stageLabelOf[source.stage]}）的结论：${verdict}`
    : "";
  return {
    ...structuredClone(source),
    stage: "in_depth",
    focus_instructions: list([...report.next_round.focus_areas, ...source.focus_instructions]),
    // 指定了题目数量时，必问题不能比它多，后端会拒绝
    must_ask_questions: list(
      report.next_round.questions.map((q) => q.question),
      Math.min(limits.max_list_items, source.question_count ?? limits.max_list_items),
    ),
    screening_checks: [],
    prior_notes: clip(
      [source.prior_notes.trim(), note].filter(Boolean).join("\n"),
      limits.max_text_chars,
    ),
  };
}
