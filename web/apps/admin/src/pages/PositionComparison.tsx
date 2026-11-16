import {
  Avatar,
  Badge,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Table,
  useToast,
} from "@ai-interview/ui";
import { Check, Copy, Printer, Scale } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { errorMessage, fieldErrors } from "../api/http";
import { useComparison, useInterviews, usePosition, useStartComparison } from "../api/queries";
import type {
  ComparedCandidate,
  ComparisonResult,
  InterviewSummary,
  Position,
  PositionComparison,
} from "../api/types";
import { Empty, Loading, QueryView, RecommendationBadge } from "../components/common";
import { PageHeader } from "../components/PageHeader";
import { hasVerdict, ReportVerdict } from "../components/ReportVerdict";
import { displayNames, groupByTier, namesInText, sameSelection } from "../lib/comparison";
import {
  confidenceLabel,
  credibilityLabel,
  credibilityTone,
  duration,
  fullTime,
  importanceLabel,
  rating,
  recommendationLabel,
  requirementVerdictLabel,
  requirementVerdictTone,
  reviewFlagLabel,
  scoreRangeText,
} from "../lib/format";
import { usePrinting } from "../lib/print";

/** 一次最少、最多比几场，和后端一致 */
const MIN_PICK = 2;
const MAX_PICK = 20;

const busy = (comparison: PositionComparison | null | undefined) =>
  comparison?.status === "pending" || comparison?.status === "running";

/** 选要比的几场面试 */
function Picker({
  position,
  interviews,
  comparison,
}: {
  position: Position;
  interviews: InterviewSummary[];
  comparison: PositionComparison | null;
}) {
  const start = useStartComparison(position.id);
  // 一开始勾上上次比的那几场；没比过就把有报告的都勾上
  const [picked, setPicked] = useState<string[]>(() => {
    const known = new Set(interviews.map((i) => i.id));
    const last = comparison?.interview_ids.filter((id) => known.has(id)) ?? [];
    return (last.length >= MIN_PICK ? last : interviews.map((i) => i.id)).slice(0, MAX_PICK);
  });
  const toggle = (id: string) =>
    setPicked((now) => (now.includes(id) ? now.filter((x) => x !== id) : [...now, id]));

  const running = busy(comparison);
  const problem =
    picked.length < MIN_PICK
      ? `至少选 ${MIN_PICK} 场`
      : picked.length > MAX_PICK
        ? `一次最多比 ${MAX_PICK} 场`
        : null;
  const again =
    Boolean(comparison?.result) && sameSelection(picked, comparison?.interview_ids ?? []);

  return (
    // 选人是在页面上做的事，打印出来的对比里不要
    <Card className="adm-no-print">
      <CardHeader
        title="选要比的人"
        actions={
          <Button
            icon={<Scale aria-hidden="true" />}
            loading={start.isPending}
            disabled={running || problem !== null}
            onClick={() => start.mutate(picked)}
          >
            {again ? "重新对比" : "开始对比"}
          </Button>
        }
      />
      <CardBody>
        {start.error && (
          <Banner tone="danger" className="adm-compare__gap">
            {fieldErrors(start.error).interview_ids ?? errorMessage(start.error)}
          </Banner>
        )}
        <p className="oil-muted adm-note">
          只列出已经有报告的面试。已选 {picked.length} 场{problem ? `，${problem}` : ""}
          。对比由大模型读这几份报告写出来，一般不到一分钟。
        </p>
        {/* biome-ignore lint/a11y/useSemanticElements: 一组可以多选的按钮，不是表单分组 */}
        <div role="group" aria-label="要比的面试" className="adm-compare__picks">
          {interviews.map((i) => {
            const on = picked.includes(i.id);
            return (
              <button
                key={i.id}
                type="button"
                className="adm-pick"
                aria-pressed={on}
                onClick={() => toggle(i.id)}
              >
                <Avatar name={i.candidate_name} />
                <span className="oil-grow">
                  <strong>{i.candidate_name}</strong>
                  <span className="adm-person__sub" style={{ display: "block" }}>
                    {fullTime(i.ended_at ?? i.started_at ?? i.created_at)} 的面试
                  </span>
                </span>
                <ReportVerdict interview={i} />
                <span className="adm-compare__tick">
                  {on && <Check aria-hidden="true" size={18} />}
                </span>
              </button>
            );
          })}
        </div>
      </CardBody>
    </Card>
  );
}

/** 一位候选人在对比里的那一块 */
function CandidateBlock({
  candidate,
  names,
}: {
  candidate: ComparedCandidate;
  names: Map<string, string>;
}) {
  const name = names.get(candidate.label) ?? candidate.label;
  const text = (s: string) => namesInText(s, names);
  const range = scoreRangeText(candidate.score_range);
  const flagged = candidate.integrity_risk === "medium" || candidate.integrity_risk === "high";
  // 诚信迹象上面单独说了，这里不再重复
  const flags = candidate.review_flags.filter((code) => !(flagged && code === "integrity_risk"));
  return (
    <li className="adm-compare__person">
      <div className="adm-compare__head">
        <Avatar name={name} />
        <Link to={`/interviews/${candidate.interview_id}?tab=report`} className="adm-link">
          {name}
        </Link>
        {typeof candidate.overall_score === "number" && (
          <span
            className="oil-num adm-verdict-brief__score"
            title={range ? `重新生成大多落在 ${range} 之间，仅供参考` : undefined}
          >
            {candidate.overall_score} 分
          </span>
        )}
        <span className="adm-item__sub">{confidenceLabel[candidate.confidence]}</span>
        <span className="oil-grow" />
        <Link
          to={`/interviews/${candidate.interview_id}?tab=report`}
          className="adm-item__sub adm-no-print"
        >
          看这份报告
        </Link>
      </div>
      <p className="adm-flat">{text(candidate.standing)}</p>
      {(candidate.close_to.length > 0 || flagged || flags.length > 0) && (
        <div className="oil-row oil-row--wrap">
          {candidate.close_to.length > 0 && (
            <Badge tone="info">
              和{candidate.close_to.map((label) => names.get(label) ?? label).join("、")}
              分不出先后
            </Badge>
          )}
          {flagged && <Badge tone="warning">有诚信方面的迹象，要人工复核</Badge>}
          {candidate.resume_credibility && candidate.resume_credibility !== "high" && (
            <Badge tone={credibilityTone[candidate.resume_credibility]} plain>
              简历：{credibilityLabel[candidate.resume_credibility]}
            </Badge>
          )}
          {flags.map((code) => (
            <Badge key={code} plain>
              {reviewFlagLabel[code]}
            </Badge>
          ))}
        </div>
      )}
      <div className="adm-compare__sides">
        <div>
          <h4>比别人强在哪</h4>
          {candidate.advantages.length ? (
            <ul>
              {candidate.advantages.map((item) => (
                <li key={item}>{text(item)}</li>
              ))}
            </ul>
          ) : (
            <p className="oil-muted adm-flat">没有写。</p>
          )}
        </div>
        <div>
          <h4>比别人弱在哪，要先核实什么</h4>
          {candidate.concerns.length ? (
            <ul>
              {candidate.concerns.map((item) => (
                <li key={item}>{text(item)}</li>
              ))}
            </ul>
          ) : (
            <p className="oil-muted adm-flat">没有写。</p>
          )}
        </div>
      </div>
    </li>
  );
}

/** 一份生成好的对比 */
export function ComparisonView({
  result,
  positionTitle,
}: {
  result: ComparisonResult;
  /** 岗位的名字。打印出来没有页面上方的导航，抬头靠它说明是哪个岗位 */
  positionTitle?: string;
}) {
  const toast = useToast();
  const root = useRef<HTMLDivElement>(null);
  // 用浏览器自己的打印（Ctrl+P）和点下面的按钮都走这里：深色主题先换成浅色
  usePrinting(root);
  const { comparison, people } = result;
  const names = useMemo(
    () => displayNames(comparison.candidates, people),
    [comparison.candidates, people],
  );
  const text = (s: string) => namesInText(s, names);
  const who = (labels: string[]) => labels.map((label) => names.get(label) ?? label).join("、");
  const copy = (content: string, done: string) => {
    void navigator.clipboard?.writeText(content).then(() => toast(done));
  };
  const groups = groupByTier(comparison.candidates);
  // 能力的名字：按第一位的顺序，别人多出来的接在后面
  const competencies = [
    ...new Set(comparison.candidates.flatMap((c) => c.competencies.map((k) => k.name))),
  ];
  const questions = comparison.tie_breakers
    .map((t, index) => `${index + 1}. ${text(t.question)}`)
    .join("\n");

  return (
    <div ref={root} className="oil-stack adm-compare">
      <p className="adm-print-only adm-flat">
        {positionTitle ? `${positionTitle}：` : ""}候选人对比，{fullTime(result.generated_at)}
        生成。由 AI 读各自的面试报告写出来，供参考。
      </p>
      <Card>
        <CardHeader
          title="结论"
          actions={
            <Button
              size="sm"
              variant="ghost"
              icon={<Printer aria-hidden="true" />}
              onClick={() => window.print()}
            >
              打印 / 存成 PDF
            </Button>
          }
        />
        <CardBody>
          <p className="adm-compare__summary">{text(comparison.summary)}</p>
          {comparison.caveats.length > 0 && (
            <Banner tone="warning" title="看这份对比要注意">
              <ul className="adm-compare__caveats">
                {comparison.caveats.map((item) => (
                  <li key={item}>{text(item)}</li>
                ))}
              </ul>
            </Banner>
          )}
        </CardBody>
      </Card>

      <Card>
        <CardHeader title="先约谁" />
        <CardBody>
          <p className="oil-muted adm-note">
            按报告的建议分成几档，档次的先后是可靠的。同一档里谁排在前面只是参考，标了「分不出先后」的几位不要按顺序取舍。
          </p>
          {groups.map((group) => (
            <section key={group.decision} className="adm-compare__tier">
              <h3>
                <RecommendationBadge value={group.decision} />
                <span className="adm-item__sub">{group.candidates.length} 位</span>
              </h3>
              <ul className="adm-compare__people" aria-label={recommendationLabel[group.decision]}>
                {group.candidates.map((c) => (
                  <CandidateBlock key={c.label} candidate={c} names={names} />
                ))}
              </ul>
            </section>
          ))}
        </CardBody>
      </Card>

      {comparison.requirements.length > 0 && (
        <Card>
          <CardHeader title="岗位要求对照" />
          <CardBody>
            <Table hover={false} className="adm-compare__table">
              <thead>
                <tr>
                  <th scope="col">要求</th>
                  {comparison.candidates.map((c) => (
                    <th key={c.label} scope="col">
                      {names.get(c.label) ?? c.label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {comparison.requirements.map((row) => (
                  <tr key={row.requirement}>
                    <th scope="row">
                      {row.requirement}
                      <span className="adm-item__sub"> {importanceLabel[row.importance]}</span>
                    </th>
                    {comparison.candidates.map((c) => {
                      const verdict = row.cells.find((cell) => cell.label === c.label)?.verdict;
                      return (
                        <td key={c.label}>
                          {verdict ? (
                            <Badge tone={requirementVerdictTone[verdict]} plain>
                              {requirementVerdictLabel[verdict]}
                            </Badge>
                          ) : (
                            <span className="oil-muted">—</span>
                          )}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </Table>
          </CardBody>
        </Card>
      )}

      {competencies.length > 0 && (
        <Card>
          <CardHeader title="各项能力的评分" />
          <CardBody>
            <p className="oil-muted adm-note">
              1 到 5 分，取自各自的报告。每个人的题目不完全一样，差一分以内不要当成差别。
            </p>
            <Table hover={false} className="adm-compare__table">
              <thead>
                <tr>
                  <th scope="col">能力</th>
                  {comparison.candidates.map((c) => (
                    <th key={c.label} scope="col">
                      {names.get(c.label) ?? c.label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {competencies.map((competency) => (
                  <tr key={competency}>
                    <th scope="row">{competency}</th>
                    {comparison.candidates.map((c) => {
                      const found = c.competencies.find((k) => k.name === competency);
                      return (
                        <td key={c.label} className="oil-num">
                          {found ? (
                            rating(found.rating ?? null)
                          ) : (
                            <span className="oil-muted">—</span>
                          )}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </Table>
          </CardBody>
        </Card>
      )}

      {comparison.differences.length > 0 && (
        <Card>
          <CardHeader title="值得注意的差别" />
          <CardBody>
            <ul className="adm-findings">
              {comparison.differences.map((d) => (
                <li key={`${d.topic}${d.detail}`}>
                  <span>{d.topic}</span>
                  {d.labels.length > 0 && <span className="adm-item__sub"> {who(d.labels)}</span>}
                  <p className="adm-flat">{text(d.detail)}</p>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>
      )}

      {comparison.tie_breakers.length > 0 && (
        <Card>
          <CardHeader
            title="人工面试时问什么能分出来"
            actions={
              <Button
                variant="secondary"
                size="sm"
                icon={<Copy aria-hidden="true" />}
                onClick={() => copy(questions, "问题都复制了")}
              >
                复制全部问题
              </Button>
            }
          />
          <CardBody>
            <ul className="adm-findings">
              {comparison.tie_breakers.map((t) => (
                <li key={t.question} className="adm-compare__question">
                  <div>
                    <span>{text(t.question)}</span>
                    <p className="adm-flat oil-muted">
                      {t.labels.length > 0 && `用来区分${who(t.labels)}。`}
                      {text(t.purpose)}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    icon={<Copy aria-hidden="true" />}
                    aria-label={`复制这个问题：${text(t.question)}`}
                    onClick={() => copy(text(t.question), "已复制")}
                  >
                    复制
                  </Button>
                </li>
              ))}
            </ul>
          </CardBody>
        </Card>
      )}

      <p className="oil-muted adm-flat adm-compare__meta">
        {fullTime(result.generated_at)}生成，用时 {duration(comparison.meta.elapsed_ms)}
        。对比只读了各自的报告，没有重新看录像；结论供参考，约谁由你决定。
      </p>
    </div>
  );
}

function Body({ position }: { position: Position }) {
  const interviews = useInterviews({ position_id: position.id, page_size: 100, sort: "-score" });
  const comparison = useComparison(position.id);
  // 换一批重新比的时候，结果区先不动；选的人只在第一次有数据时定下来
  const [ready, setReady] = useState(false);
  useEffect(() => {
    if (interviews.data !== undefined && comparison.data !== undefined) setReady(true);
  }, [interviews.data, comparison.data]);

  if (comparison.error && comparison.data === undefined)
    return <Banner tone="danger">{errorMessage(comparison.error)}</Banner>;
  if (!ready || interviews.data === undefined || comparison.data === undefined) return <Loading />;

  const eligible = interviews.data.items.filter(hasVerdict);
  const current = comparison.data;
  if (eligible.length < MIN_PICK)
    return (
      <Empty
        title="还不够两个人"
        description="要有两场以上出了报告的面试才能对比。等候选人面完、报告生成好再来。"
        actions={
          <Link to={`/positions/${position.id}`} className="adm-link">
            回到岗位
          </Link>
        }
      />
    );

  return (
    <div className="oil-stack" style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}>
      <Picker position={position} interviews={eligible} comparison={current} />
      {busy(current) && (
        <Banner tone="info" title="正在对比">
          一般不到一分钟，好了会自己显示出来。{current?.result && "下面是上一次的对比。"}
        </Banner>
      )}
      {current?.status === "failed" && (
        <Banner tone="danger" title="这次对比没有生成出来">
          {current.error ?? "原因不明。"}可以再点一次「开始对比」。
          {current.result && "下面是上一次的对比。"}
        </Banner>
      )}
      {current?.result ? (
        <ComparisonView result={current.result} positionTitle={position.title} />
      ) : (
        !busy(current) &&
        current?.status !== "failed" && (
          <p className="oil-muted adm-flat">
            选好人以后点「开始对比」。结果会告诉你建议先约谁、每个人强在哪弱在哪、岗位要求各自满足得怎么样，还有分不出高下时人工面试可以问什么。
          </p>
        )
      )}
    </div>
  );
}

/** 同一个岗位几位候选人的横向对比 */
export function PositionComparisonPage() {
  const { id } = useParams();
  const position = usePosition(id);
  return (
    <QueryView query={position}>
      {(p) => (
        <>
          <PageHeader
            crumbs={
              <>
                <Link to="/positions">岗位</Link>
                <span aria-hidden="true">/</span>
                <Link to={`/positions/${p.id}`}>{p.title}</Link>
              </>
            }
            title="对比候选人"
            tabTitle={`对比候选人 · ${p.title}`}
            description="把这个岗位面完的几位放在一起看，决定先约谁做人工面试。"
          />
          <Body position={p} />
        </>
      )}
    </QueryView>
  );
}
