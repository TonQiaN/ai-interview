/**
 * 分析报告。结构照 crates/interview-report 的 InterviewReport。
 *
 * 几条展示上的约定（和分析报告线对过）：
 * - 需要人工复核的提示放最上面。
 * - 建议和理由是主角；综合分重新生成会有几分的浮动，只作参考，不做成最大的数字。
 * - 每个判断下面跟着原话，点一下跳到回看的那个时间。简历里的原文没有时间，不能点。
 * - 诚信观察是「值得去核对的迹象」，每条都带着别的可能解释，不写成结论。
 * - 评分为空表示没考察到，不是 0 分。
 */
import { Badge, Banner, Button, Card, CardBody, CardHeader, ScoreBar } from "@ai-interview/ui";
import {
  ChevronDown,
  FileText,
  Flag,
  ListChecks,
  MousePointerClick,
  RefreshCw,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import type {
  AspectRating,
  ClaimCheck,
  CompetencyRating,
  Evidence,
  Finding,
  FocusCheck,
  IntegritySignal,
  InterviewReport,
  QuestionReview,
  RequirementCheck,
} from "../api/types";
import {
  aspectLabel,
  basisLabel,
  claimVerdictLabel,
  claimVerdictTone,
  clock,
  confidenceLabel,
  credibilityLabel,
  credibilityTone,
  duration,
  eventEvidenceText,
  focusVerdictLabel,
  focusVerdictTone,
  fullTime,
  importanceLabel,
  levelLabel,
  levelTone,
  matchLevelLabel,
  matchLevelTone,
  rating,
  recommendationLabel,
  recommendationTone,
  requirementVerdictLabel,
  requirementVerdictTone,
  resumeQuote,
  reviewFlagLabel,
  riskLabel,
  riskTone,
  signalKindLabel,
  speakerLabel,
  stageName,
} from "../lib/format";

type Seek = (ms: number) => void;

const matchNote: Partial<Record<NonNullable<Evidence["quote_match"]>, string>> = {
  approximate: "大意，和原话不完全一样",
  whole_segment: "引的是整段话",
  unchecked: "没有核对过原话",
};

function Quote({ evidence, onSeek }: { evidence: Evidence; onSeek: Seek }) {
  const note = evidence.quote_match ? matchNote[evidence.quote_match] : undefined;
  const mark = note && (
    <span className="adm-quote__mark" title={note}>
      {evidence.quote_match === "approximate"
        ? "大意"
        : evidence.quote_match === "whole_segment"
          ? "整段"
          : "未核对"}
    </span>
  );

  if (evidence.source === "resume") {
    return (
      <div className="adm-quote adm-quote--static">
        <span className="adm-quote__time">
          <FileText aria-hidden="true" size={12} /> 简历
        </span>
        <span>
          “{resumeQuote(evidence.quote)}”{mark}
        </span>
      </div>
    );
  }

  const text =
    evidence.source === "event" ? eventEvidenceText(evidence.quote) : `“${evidence.quote}”`;
  const who =
    evidence.source === "event"
      ? "过程记录"
      : evidence.speaker && evidence.speaker !== "candidate"
        ? speakerLabel[evidence.speaker]
        : null;
  const start = evidence.start_ms;
  if (start === undefined || start === null) {
    return (
      <div className="adm-quote adm-quote--static">
        <span>
          {who && <span className="adm-quote__who">{who}</span>}
          {text}
          {mark}
        </span>
      </div>
    );
  }
  return (
    <button
      type="button"
      className="adm-quote"
      onClick={() => onSeek(start)}
      title="跳到回看的这个位置"
    >
      <span className="adm-quote__time">{clock(start)}</span>
      <span>
        {who && <span className="adm-quote__who">{who}</span>}
        {text}
        {mark}
      </span>
    </button>
  );
}

/** 一个判断的出处。多于 limit 条时先收起来 */
function Quotes({ items, onSeek, limit = 2 }: { items: Evidence[]; onSeek: Seek; limit?: number }) {
  const [open, setOpen] = useState(false);
  if (items.length === 0) return null;
  const shown = open ? items : items.slice(0, limit);
  return (
    <div className="adm-quotes">
      {shown.map((e, index) => (
        // 同一个判断下可能两次引同一句话，用位置当 key
        // biome-ignore lint/suspicious/noArrayIndexKey: 出处列表是只读的，顺序不会变
        <Quote key={index} evidence={e} onSeek={onSeek} />
      ))}
      {items.length > limit && (
        <button type="button" className="adm-link adm-quotes__more" onClick={() => setOpen(!open)}>
          {open ? "收起" : `还有 ${items.length - limit} 处出处`}
        </button>
      )}
    </div>
  );
}

function Findings({ items, onSeek, empty }: { items: Finding[]; onSeek: Seek; empty: string }) {
  if (items.length === 0) return <p className="oil-muted adm-flat">{empty}</p>;
  return (
    <ul className="adm-findings">
      {items.map((f) => (
        <li key={f.point}>
          <span>{f.point}</span>
          <Quotes items={f.evidence} onSeek={onSeek} />
        </li>
      ))}
    </ul>
  );
}

function Section({
  title,
  nav,
  aside,
  intro,
  children,
}: {
  title: string;
  /** 目录里显示的短名字，不给就用标题；false 表示不进目录 */
  nav?: string | false;
  aside?: ReactNode;
  intro?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section
      className="adm-report__section"
      data-report-section={nav === false ? undefined : (nav ?? title)}
    >
      <Card>
        <CardHeader title={title} actions={aside} />
        <CardBody>
          {intro && <p className="adm-report-intro">{intro}</p>}
          {children}
        </CardBody>
      </Card>
    </section>
  );
}

/** 这一部分没生成出来时的占位 */
function Missing({ title, nav }: { title: string; nav?: string }) {
  return (
    <Section title={title} nav={nav}>
      <p className="oil-muted adm-flat">这部分分析没有生成出来，可以重新生成报告。</p>
    </Section>
  );
}

/**
 * 报告很长，顶上放一排目录，滚动时一直贴在上面。
 * 目录是从页面上实际渲染出来的部分读的，哪部分没有就不会出现在目录里。
 */
function ReportNav({
  root,
  version,
}: {
  root: React.RefObject<HTMLElement | null>;
  version: string;
}) {
  const [names, setNames] = useState<string[]>([]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: 换了一份报告要重新读一遍目录
  useEffect(() => {
    const found = root.current?.querySelectorAll<HTMLElement>("[data-report-section]") ?? [];
    setNames(Array.from(found, (el) => el.dataset.reportSection ?? ""));
  }, [root, version]);
  if (names.length < 3) return null;
  return (
    <nav className="adm-report-nav" aria-label="报告目录">
      {names.map((name) => (
        <button
          key={name}
          type="button"
          onClick={() =>
            root.current
              ?.querySelector(`[data-report-section="${name}"]`)
              ?.scrollIntoView({ behavior: "smooth", block: "start" })
          }
        >
          {name}
        </button>
      ))}
    </nav>
  );
}

function Rated({ label, value }: { label: string; value?: number | null }) {
  if (value === null || value === undefined) {
    return (
      <div className="adm-rated">
        <span className="adm-rated__label">{label}</span>
        <Badge plain>没考察到</Badge>
      </div>
    );
  }
  return <ScoreBar label={label} value={value} max={5} />;
}

function Competency({ item, onSeek }: { item: CompetencyRating; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <Rated label={item.name} value={item.rating} />
      <p>{item.comment}</p>
      <Quotes items={item.evidence} onSeek={onSeek} />
    </div>
  );
}

function Focus({ item, onSeek }: { item: FocusCheck; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <div className="adm-dim__head">
        <strong>{item.focus}</strong>
        <Badge tone={focusVerdictTone[item.verdict]}>{focusVerdictLabel[item.verdict]}</Badge>
      </div>
      <p>{item.finding}</p>
      <Quotes items={item.evidence} onSeek={onSeek} />
    </div>
  );
}

function Requirement({ item, onSeek }: { item: RequirementCheck; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <div className="adm-dim__head">
        <strong>
          <span className={item.importance === "must_have" ? "adm-tag adm-tag--must" : "adm-tag"}>
            {importanceLabel[item.importance]}
          </span>
          {item.requirement}
        </strong>
        <Badge tone={requirementVerdictTone[item.verdict]}>
          {requirementVerdictLabel[item.verdict]}
        </Badge>
      </div>
      <p>
        {item.reasoning}
        <span className="adm-basis">{basisLabel[item.basis]}</span>
      </p>
      <Quotes items={item.evidence} onSeek={onSeek} />
    </div>
  );
}

function Claim({ item, onSeek }: { item: ClaimCheck; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <div className="adm-dim__head">
        <strong>简历写：{item.claim}</strong>
        <Badge tone={claimVerdictTone[item.verdict]}>{claimVerdictLabel[item.verdict]}</Badge>
      </div>
      <p>{item.reasoning}</p>
      <Quotes items={item.evidence} onSeek={onSeek} />
      {item.follow_up && <p className="adm-followup">下一轮可以问：{item.follow_up}</p>}
    </div>
  );
}

function Signal({ item, onSeek }: { item: IntegritySignal; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <div className="adm-dim__head">
        <strong>{signalKindLabel[item.kind] ?? item.kind}</strong>
        <span className="oil-row oil-row--wrap">
          <Badge tone={levelTone[item.severity]} plain>
            严重程度{levelLabel[item.severity]}
          </Badge>
          <Badge plain>这条迹象的可靠程度{levelLabel[item.reliability]}</Badge>
        </span>
      </div>
      <p>{item.description}</p>
      <p className="adm-alt">
        <strong>也可能是：</strong>
        {item.alternative_explanations}
      </p>
      <Quotes items={item.evidence} onSeek={onSeek} limit={3} />
    </div>
  );
}

function Aspect({ item, onSeek }: { item: AspectRating; onSeek: Seek }) {
  return (
    <div className="adm-dim">
      <Rated label={aspectLabel[item.aspect]} value={item.rating} />
      <p>{item.observation}</p>
      <Quotes items={item.evidence} onSeek={onSeek} />
    </div>
  );
}

function Question({
  item,
  index,
  failed,
  onSeek,
}: {
  item: QuestionReview;
  index: number;
  failed: boolean;
  onSeek: Seek;
}) {
  const j = item.judgement;
  const t = item.timing;
  let badge: ReactNode;
  if (!item.asked) badge = <Badge plain>没问到</Badge>;
  else if (!j) badge = <Badge tone="danger">{failed ? "点评失败" : "没有点评"}</Badge>;
  else if (!j.answered) badge = <Badge tone="warning">没有作答</Badge>;
  else
    badge = (
      <Badge tone={j.rating >= 4 ? "success" : j.rating >= 3 ? "warning" : "danger"}>
        {rating(j.rating)}
      </Badge>
    );

  const facts = item.asked
    ? [
        `说了 ${duration(t.candidate_speaking_ms)}`,
        typeof t.response_latency_ms === "number" &&
          `开口前停了 ${(t.response_latency_ms / 1000).toFixed(1)} 秒`,
        typeof t.chars_per_minute === "number" && `每分钟 ${Math.round(t.chars_per_minute)} 字`,
        typeof t.longest_pause_ms === "number" &&
          `中间最长停顿 ${(t.longest_pause_ms / 1000).toFixed(1)} 秒`,
      ].filter(Boolean)
    : [];

  return (
    <details className="adm-qa">
      <summary>
        <span className="adm-qa__no">{index + 1}</span>
        <span>{item.question_text}</span>
        <span className="adm-qa__end">
          {badge}
          <ChevronDown aria-hidden="true" size={16} className="adm-qa__chevron" />
        </span>
      </summary>
      <div className="adm-qa__body">
        {!item.asked && <p>面试结束前没有问到这道题。</p>}
        {item.asked && !j && <p>这道题的点评没有生成出来，可以重新生成报告。</p>}
        {item.asked && (
          <p className="adm-qa__facts">
            <button type="button" className="adm-link" onClick={() => onSeek(t.start_ms)}>
              从 {clock(t.start_ms)} 开始回看
            </button>
            {facts.map((fact) => (
              <span key={String(fact)}>{fact}</span>
            ))}
          </p>
        )}
        {j && (
          <>
            <p>{j.answer_summary}</p>
            <p>
              <strong>为什么是这个分：</strong>
              {j.rating_reason}
            </p>
            {j.strengths.length > 0 && (
              <>
                <h4 className="adm-qa__h">答得好的</h4>
                <Findings items={j.strengths} onSeek={onSeek} empty="" />
              </>
            )}
            {j.weaknesses.length > 0 && (
              <>
                <h4 className="adm-qa__h">不足的</h4>
                <Findings items={j.weaknesses} onSeek={onSeek} empty="" />
              </>
            )}
            {j.follow_up && <p className="adm-followup">下一轮可以问：{j.follow_up}</p>}
          </>
        )}
      </div>
    </details>
  );
}

export interface ReportViewProps {
  report: InterviewReport;
  /** 点出处时跳到回看的这个时间 */
  onSeek: Seek;
  /** 有它才显示「重新生成」 */
  onRegenerate?: () => void;
}

export function ReportView({ report, onSeek, onRegenerate }: ReportViewProps) {
  const rec = report.recommendation;
  const failed = report.meta.failed_stages;
  const failedStage = (stage: string) => failed.some((f) => f.stage === stage);
  const t = report.timing;
  const usage = report.meta.usage;
  const cited = report.meta.evidence;
  const root = useRef<HTMLDivElement>(null);
  const regenerate = onRegenerate && (
    <Button
      size="sm"
      variant="secondary"
      icon={<RefreshCw aria-hidden="true" />}
      onClick={onRegenerate}
    >
      重新生成
    </Button>
  );

  return (
    <div
      ref={root}
      className="oil-stack adm-report"
      style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
    >
      <ReportNav root={root} version={`${report.interview_id}:${report.generated_at_ms}`} />
      {failed.length > 0 && (
        <Banner tone="danger" title="有几部分分析没有成功" actions={regenerate}>
          {failed.map((f) => stageName(f.stage)).join("、")}
          没有生成出来，下面对应的位置是空的。可以重新生成报告。
        </Banner>
      )}

      {report.review_flags.length > 0 && (
        <Banner tone="warning" title="下结论前，建议你先看这几处">
          <ul className="adm-flags">
            {report.review_flags.map((flag) => (
              <li key={flag.code}>
                <Flag aria-hidden="true" size={14} />
                <span>
                  <strong>{reviewFlagLabel[flag.code] ?? flag.code}</strong>
                  {flag.message}
                </span>
              </li>
            ))}
          </ul>
        </Banner>
      )}

      <Card>
        <CardBody>
          <div className="oil-row oil-row--wrap adm-verdict__badges">
            <Badge tone={recommendationTone[rec.decision]}>
              {recommendationLabel[rec.decision]}
            </Badge>
            <Badge plain>{confidenceLabel[rec.confidence]}</Badge>
            {report.integrity && (
              <Badge tone={riskTone[report.integrity.risk_level]} plain>
                诚信风险：{riskLabel[report.integrity.risk_level]}
              </Badge>
            )}
          </div>
          <h2 className="adm-verdict__headline">{rec.headline}</h2>
          <p className="adm-verdict__overview">{rec.overview}</p>
          <dl className="adm-facts">
            <div>
              <dt>综合分</dt>
              <dd>
                {report.overall_score === null ? "没有评分" : `${report.overall_score} / 100`}
                <small>重新生成会有几分的浮动，仅供参考</small>
              </dd>
            </div>
            <div>
              <dt>时长</dt>
              <dd>
                {duration(t.duration_ms)}
                <small>预计 {duration(t.expected_ms)}</small>
              </dd>
            </div>
            <div>
              <dt>题目</dt>
              <dd>
                问了 {t.questions_asked} / {t.questions_planned} 道
                <small>作答 {t.questions_answered} 道</small>
              </dd>
            </div>
            <div>
              <dt>候选人说话</dt>
              <dd>
                {duration(t.candidate_speaking_ms)}
                <small>面试官 {duration(t.interviewer_speaking_ms)}</small>
              </dd>
            </div>
          </dl>
          <p className="adm-report-note">
            <MousePointerClick aria-hidden="true" size={14} />
            这是给你参考的初筛意见，决定由你来做。每条判断下面是候选人的原话，点一下可以跳到回看里听。
          </p>
        </CardBody>
      </Card>

      <div
        className="adm-split adm-split--even adm-report__section"
        data-report-section="理由和顾虑"
      >
        <Section title="主要理由" nav={false}>
          <Findings items={rec.key_reasons} onSeek={onSeek} empty="没有列出理由。" />
        </Section>
        <Section title="风险和顾虑" nav={false}>
          <Findings items={rec.risks} onSeek={onSeek} empty="没有明显的顾虑。" />
        </Section>
      </div>

      {report.focus_checks.length > 0 && (
        <Section
          title="你要求重点考察的"
          nav="重点考察"
          intro="安排面试时写下的考察重点，逐条看有没有考察到、结果怎样。"
        >
          {report.focus_checks.map((item) => (
            <Focus key={item.focus} item={item} onSeek={onSeek} />
          ))}
        </Section>
      )}

      <Section title="能力评分" intro="每项 1 到 5 分。没有考察到的不打分。">
        {report.competencies.map((item) => (
          <Competency key={item.name} item={item} onSeek={onSeek} />
        ))}
      </Section>

      {report.requirement_match ? (
        <Section
          title="和岗位要求对照"
          nav="岗位要求"
          aside={
            <Badge tone={matchLevelTone[report.requirement_match.match_level]}>
              {matchLevelLabel[report.requirement_match.match_level]}
            </Badge>
          }
          intro={report.requirement_match.summary}
        >
          {report.requirement_match.items.map((item) => (
            <Requirement key={item.requirement} item={item} onSeek={onSeek} />
          ))}
        </Section>
      ) : (
        failedStage("requirements") && <Missing title="和岗位要求对照" nav="岗位要求" />
      )}

      {report.resume_verification ? (
        <Section
          title="简历上的经历是不是真的"
          nav="简历核实"
          aside={
            <Badge tone={credibilityTone[report.resume_verification.credibility]}>
              {credibilityLabel[report.resume_verification.credibility]}
            </Badge>
          }
          intro={report.resume_verification.summary}
        >
          {report.resume_verification.claims.map((item) => (
            <Claim key={item.claim} item={item} onSeek={onSeek} />
          ))}
        </Section>
      ) : (
        failedStage("resume") && <Missing title="简历上的经历是不是真的" nav="简历核实" />
      )}

      {report.integrity ? (
        <Section
          title="诚信观察"
          aside={
            <Badge tone={riskTone[report.integrity.risk_level]}>
              风险：{riskLabel[report.integrity.risk_level]}
            </Badge>
          }
          intro={report.integrity.summary}
        >
          {report.integrity.signals.length === 0 ? (
            <p className="oil-muted adm-flat">没有发现值得注意的迹象。</p>
          ) : (
            <>
              <p className="adm-caution">
                下面是值得你去回看里核对的迹象，不是结论。每一条都写了别的可能解释。
              </p>
              {report.integrity.signals.map((item) => (
                <Signal key={`${item.kind}-${item.description}`} item={item} onSeek={onSeek} />
              ))}
            </>
          )}
        </Section>
      ) : (
        failedStage("integrity") && <Missing title="诚信观察" />
      )}

      {report.communication ? (
        <Section title="表达和状态" intro={report.communication.summary}>
          {report.communication.aspects.map((item) => (
            <Aspect key={item.aspect} item={item} onSeek={onSeek} />
          ))}
          {report.communication.reliability_note && (
            <p className="adm-caution adm-caution--end">{report.communication.reliability_note}</p>
          )}
        </Section>
      ) : (
        failedStage("communication") && <Missing title="表达和状态" />
      )}

      <Section title="逐题点评" intro="追问和它所属的题目算在一起。点开看每道题的回答和评分理由。">
        {report.questions.map((item, index) => (
          <Question
            key={item.question_id}
            item={item}
            index={index}
            failed={failedStage(`question:${item.question_id}`)}
            onSeek={onSeek}
          />
        ))}
      </Section>

      {(report.next_round.focus_areas.length > 0 || report.next_round.questions.length > 0) && (
        <Section
          title="下一轮可以接着问"
          nav="下一轮"
          aside={<ListChecks aria-hidden="true" size={18} />}
        >
          {report.next_round.focus_areas.length > 0 && (
            <ul className="adm-bullets">
              {report.next_round.focus_areas.map((area) => (
                <li key={area}>{area}</li>
              ))}
            </ul>
          )}
          {report.next_round.questions.length > 0 && (
            <ol className="adm-next">
              {report.next_round.questions.map((q) => (
                <li key={q.question}>
                  <span>{q.question}</span>
                  <small>{q.purpose}</small>
                </li>
              ))}
            </ol>
          )}
        </Section>
      )}

      {report.limitations.length > 0 && (
        <Section title="这份报告的局限" nav="局限">
          <ul className="adm-bullets adm-bullets--muted">
            {report.limitations.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </Section>
      )}

      <details className="adm-meta">
        <summary>这份报告是怎么生成的</summary>
        <dl className="adm-facts adm-facts--small">
          <div>
            <dt>生成时间</dt>
            <dd>{fullTime(new Date(report.generated_at_ms).toISOString())}</dd>
          </div>
          <div>
            <dt>模型</dt>
            <dd>
              {report.meta.model}
              <small>思考强度 {report.meta.thinking}</small>
            </dd>
          </div>
          <div>
            <dt>用量</dt>
            <dd>
              用时 {duration(report.meta.elapsed_ms)}
              <small>
                输入 {usage.prompt_tokens.toLocaleString("zh-CN")}、输出{" "}
                {usage.completion_tokens.toLocaleString("zh-CN")} token
              </small>
            </dd>
          </div>
          <div>
            <dt>出处核对</dt>
            <dd>
              引用 {cited.cited} 处，{cited.exact} 处和原话逐字对上
              <small>
                大意 {cited.approximate} 处，整段 {cited.whole_segment} 处，对不上被丢掉{" "}
                {cited.dropped} 处
              </small>
            </dd>
          </div>
        </dl>
        {regenerate && <div className="adm-meta__actions">{regenerate}</div>}
      </details>
    </div>
  );
}
