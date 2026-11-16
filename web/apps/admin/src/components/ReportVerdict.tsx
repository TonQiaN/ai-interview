import { Badge } from "@ai-interview/ui";
import type { InterviewSummary } from "../api/types";
import { reportStatusLabel, scoreRangeText } from "../lib/format";
import { DecisionBadge, InterviewStatusBadge, RecommendationBadge, StageBadge } from "./common";

type Props = {
  interview: Pick<
    InterviewSummary,
    "report_status" | "recommendation" | "overall_score" | "score_range"
  >;
};

/** 列表里的一场面试有没有报告的结论或分数 */
export function hasVerdict(interview: Props["interview"]): boolean {
  return Boolean(interview.recommendation) || typeof interview.overall_score === "number";
}

/**
 * 列表里一场面试的报告：有结论和综合分就直接显示，HR 不用点进去看；
 * 还没有的话说报告到哪一步了，没开始面试的显示一条横线。
 *
 * 结论和分数取自最新一份生成好的报告，所以重新生成的那一份还没好（或者没成功）时，
 * 显示的还是上一份的，旁边说一声。
 */
export function ReportVerdict({ interview }: Props) {
  const status = interview.report_status;
  if (!hasVerdict(interview)) {
    if (!status) return <span className="oil-muted">—</span>;
    if (status === "done") return <Badge tone="success">已生成</Badge>;
    if (status === "failed") return <Badge tone="danger">{reportStatusLabel.failed}</Badge>;
    return <Badge tone="info">{reportStatusLabel[status]}</Badge>;
  }
  const score = interview.overall_score;
  const range = scoreRangeText(interview.score_range);
  return (
    <span className="adm-verdict-brief">
      <RecommendationBadge value={interview.recommendation ?? null} />
      {typeof score === "number" && (
        <span
          className="oil-num adm-verdict-brief__score"
          title={range ? `重新生成大多落在 ${range} 之间，仅供参考` : undefined}
        >
          {score} 分
        </span>
      )}
      {(status === "pending" || status === "running") && (
        <span className="adm-item__sub">新的一份在生成</span>
      )}
      {status === "failed" && <span className="adm-item__sub">新的一份没生成出来</span>}
    </span>
  );
}

/**
 * 窄列表里一场面试右边的那几个标签：有报告的结论就显示结论、分数和 HR 的决定，
 * 没有就说面试到哪一步了（有了结论，「已完成」就不用再说一遍）。
 */
export function InterviewTags({ interview }: { interview: InterviewSummary }) {
  return (
    <span className="adm-item__tags">
      {hasVerdict(interview) && <ReportVerdict interview={interview} />}
      <DecisionBadge value={interview.decision} />
      {!hasVerdict(interview) && <InterviewStatusBadge interview={interview} />}
      <StageBadge stage={interview.stage} />
      {interview.archived_at && <Badge plain>已归档</Badge>}
    </span>
  );
}
