/**
 * RAAS 里投递走到哪一步了。RAAS 给的是它自己的英文代号，这里换成它页面上用的中文说法；
 * 不认识的代号原样显示，不猜。
 */
const stageLabel: Record<string, string> = {
  resume_intake: "简历入库",
  job_matching: "岗位匹配",
  internal_interview: "内部面试",
  resume_polish: "简历优化",
  hsm_review: "审核",
  client_push: "推送客户",
  client_screening: "客户筛选",
  client_interview: "客户面试",
  offer: "Offer",
  onboarding: "入职",
};

const statusLabel: Record<string, string> = {
  pending: "待处理",
  submitted: "已提交",
  pushed: "待筛选",
  client_screening: "待筛选",
  awaiting_client_feedback: "待筛选",
  screening_passed: "筛选通过",
  screening_rejected: "筛选不通过",
  screening_abandoned: "废弃",
  pending_internal_interview: "待内部面试",
  pending_interview: "待面试",
  interview_invited: "已邀约",
  interview_scheduled: "已约面试",
  interviewing: "面试中",
  interview_done: "面试完成",
  interview_passed: "面试通过",
  interview_failed: "面试不通过",
  gave_up_interview: "放弃面试",
  pending_decision: "待定",
  offer_approving: "Offer 审批中",
  offer_pending_issue: "待发 Offer",
  offer_issued: "已发 Offer",
  offer_rejected: "Offer 被拒绝",
  offer_expired: "Offer 已失效",
  pending_onboard: "待入职",
  pending_onboarding: "待入职",
  onboarding: "入职办理中",
  onboarding_delayed: "延期入职",
  onboarded: "已入职",
  gave_up_onboard: "放弃入职",
  onboarding_failed: "入职失败",
  gave_up_waiting: "放弃等待",
  abandoned: "废弃",
  rejected: "未通过",
  auto_closed: "需求关闭自动结束",
  closed: "已结束",
};

/** 「客户面试 · 面试通过」这样的一句。两个都没有就是 null */
export function raasApplicationText(
  stage: string | null | undefined,
  status: string | null | undefined,
): string | null {
  const parts = [
    stage ? (stageLabel[stage] ?? stage) : null,
    status ? (statusLabel[status] ?? status) : null,
  ].filter((part): part is string => Boolean(part));
  // 阶段和状态说的是同一件事时只说一遍（比如都是「待筛选」）
  const unique = parts.filter((part, index) => parts.indexOf(part) === index);
  return unique.length > 0 ? unique.join(" · ") : null;
}

/** RAAS 自己算的匹配分，取整。没有就是 null */
export function raasScoreText(score: number | null | undefined): string | null {
  return typeof score === "number" && Number.isFinite(score)
    ? `RAAS 匹配 ${Math.round(score)} 分`
    : null;
}
