import type { InterviewConfig, TimeBudget } from "../api/types";

/**
 * 按面试设置估算时间怎么分、会出几道题。
 *
 * 算法照 crates/interview-engine/src/config.rs 的 TimeBudget::for_config 写。
 * 这里只用来在填表时给一个即时的估计（以及演示数据），保存之后以后端算出来的
 * `time_budget` 为准。
 */
export function estimateBudget(config: InterviewConfig): TimeBudget {
  const total = config.target_duration_minutes * 60;
  const opening = 40;
  const closing = config.candidate_questions === "collect" ? 85 : 25;
  const overhead = 14 + 3 + config.answer.silence_sec;
  const usable = Math.max(0, total - opening - closing);
  const followUp =
    config.follow_ups.enabled && config.follow_ups.max_per_question > 0
      ? Math.floor((usable * 20) / 100)
      : 0;
  const forQuestions = usable - followUp;
  const perQuestion = overhead + 100;
  const wanted =
    config.question_count ?? Math.floor((forQuestions + Math.floor(perQuestion / 2)) / perQuestion);
  const count = Math.min(30, Math.max(1, config.must_ask_questions.length, wanted));
  return {
    total_sec: total,
    opening_sec: opening,
    closing_sec: closing,
    per_question_overhead_sec: overhead,
    follow_up_sec: followUp,
    question_count: count,
    answer_sec: Math.max(0, forQuestions - count * overhead),
    reserve_question_count:
      config.question_count === null ? Math.min(4, Math.max(1, Math.ceil(count / 3))) : 0,
  };
}
