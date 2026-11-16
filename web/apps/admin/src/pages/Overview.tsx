import { Button, buttonClass, Card, CardBody, CardHeader, Hero, Stat } from "@ai-interview/ui";
import { ArrowRight, Check, Plus } from "lucide-react";
import { Link, useNavigate } from "react-router";
import { useCandidates, useInterviews, useMe, usePositions, useSystemStatus } from "../api/queries";
import type { InterviewSummary } from "../api/types";
import { Person, QueryView } from "../components/common";
import { PageTitle } from "../components/PageHeader";
import { hasVerdict, InterviewTags, ReportVerdict } from "../components/ReportVerdict";
import { SystemStatusList } from "../components/SystemStatusList";
import { relativeTime } from "../lib/format";

/** 需要 HR 动手的面试，和该做的事 */
function todoOf(i: InterviewSummary): string | null {
  if (i.status === "prepare_failed") return "题目没准备好，去重试";
  if (i.status === "draft") return "还没生成题目";
  if (i.status === "ready") return "可以发链接了";
  const ended = i.status === "completed" || i.status === "abandoned";
  if (ended && i.report_status === "failed") return "报告生成失败，去重新生成";
  if (ended && i.report_status === "done" && i.decision === "pending") return "报告好了，等你决定";
  return null;
}

function greeting(now = new Date()): string {
  const h = now.getHours();
  if (h < 6) return "夜深了";
  if (h < 12) return "早上好";
  if (h < 18) return "下午好";
  return "晚上好";
}

/**
 * 一场面试都还没有的时候（刚装好，或者刚开始用），告诉 HR 先做什么。
 * 岗位和候选人已经有了的那一步标成做完，不用再点进去看。
 */
function FirstSteps() {
  const positions = usePositions({ page_size: 1 });
  const candidates = useCandidates({ page_size: 1 });
  const positionCount = positions.data?.total ?? 0;
  const candidateCount = candidates.data?.total ?? 0;
  const steps = [
    {
      title: "加一个岗位",
      description: "写清楚这个岗位要什么样的人，面试官按它出题。",
      done: positionCount > 0 ? `已经有 ${positionCount} 个岗位` : null,
      to: "/positions",
      action: "去加岗位",
    },
    {
      title: "上传候选人的简历",
      description: "PDF、Word、图片都行，系统会读出候选人的经历。",
      done: candidateCount > 0 ? `已经有 ${candidateCount} 位候选人` : null,
      to: "/candidates",
      action: "去上传简历",
    },
    {
      title: "安排面试，把链接发给候选人",
      description: "候选人打开链接就能面试，不用注册和登录。面完你会看到录像和分析报告。",
      done: null,
      to: "/interviews/new",
      action: "安排面试",
    },
  ];
  // 前面两步都做完了，第三步才是眼下该点的
  const next = steps.findIndex((s) => !s.done);
  return (
    <Card>
      <CardHeader title="三步安排第一场面试" />
      <CardBody>
        <ol className="adm-list" aria-label="三步安排第一场面试">
          {steps.map((s, i) => (
            <li key={s.title} className="adm-item adm-item--wrap">
              <span className="adm-step__dot" data-done={s.done ? "" : undefined}>
                {s.done ? <Check aria-hidden="true" size={16} /> : i + 1}
              </span>
              <div className="adm-item__main">
                <strong>{s.title}</strong>
                <div className="adm-item__sub">{s.description}</div>
              </div>
              {s.done && <span className="adm-item__sub">{s.done}</span>}
              <Link
                to={s.to}
                className={buttonClass({
                  size: "sm",
                  variant: i === next ? "primary" : "secondary",
                })}
              >
                {s.action}
              </Link>
            </li>
          ))}
        </ol>
      </CardBody>
    </Card>
  );
}

export function OverviewPage() {
  const navigate = useNavigate();
  const me = useMe();
  const interviews = useInterviews({ page_size: 100 });
  const system = useSystemStatus();

  return (
    <>
      <PageTitle title="总览" />
      <Hero painting="dusk">
        <div className="oil-eyebrow">{greeting()}</div>
        <h1 className="oil-title-xl" style={{ margin: "var(--oil-space-2) 0" }}>
          {me.data?.display_name}，今天要面谁？
        </h1>
        <p style={{ margin: "0 0 var(--oil-space-5)", maxWidth: "36em" }}>
          选一位候选人和一个岗位，AI 面试官会按简历和岗位要求出题，准备好之后把链接发给候选人就行。
        </p>
        <Button
          variant="gold"
          icon={<Plus aria-hidden="true" />}
          onClick={() => navigate("/interviews/new")}
        >
          安排一场面试
        </Button>
      </Hero>

      <QueryView query={interviews}>
        {({ items }) => {
          const todos = items.filter((i) => todoOf(i));
          const live = items.filter((i) => i.status === "in_progress").length;
          const waiting = items.filter((i) => i.status === "invited").length;
          const done = items.filter((i) => i.status === "completed");
          const advanced = items.filter((i) => i.decision === "advance").length;
          return (
            <>
              {items.length === 0 && (
                <div className="adm-section">
                  <FirstSteps />
                </div>
              )}
              <div className="adm-stats adm-section">
                <Stat label="等你处理" value={todos.length} unit="场" />
                <Stat label="正在面试" value={live} unit="场" />
                <Stat label="已发链接，等候选人" value={waiting} unit="场" />
                <Stat label="已完成" value={done.length} unit="场" />
                <Stat label="决定进入下一轮" value={advanced} unit="人" />
              </div>

              <div className="adm-split adm-section">
                <Card>
                  <CardHeader title="等你处理" />
                  <CardBody>
                    {todos.length === 0 ? (
                      <p className="oil-muted" style={{ margin: 0 }}>
                        没有要处理的事。
                      </p>
                    ) : (
                      <ul className="adm-list">
                        {todos.map((i) => (
                          <li key={i.id} className="adm-item">
                            <div className="adm-item__main">
                              <Person
                                name={i.candidate_name}
                                sub={i.position_title}
                                to={`/interviews/${i.id}`}
                              />
                            </div>
                            {hasVerdict(i) && <ReportVerdict interview={i} />}
                            <span className="adm-item__sub">{todoOf(i)}</span>
                            <Button
                              size="sm"
                              variant="ghost"
                              iconOnly
                              aria-label={`打开 ${i.candidate_name} 的面试`}
                              onClick={() => navigate(`/interviews/${i.id}`)}
                            >
                              <ArrowRight aria-hidden="true" />
                            </Button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </CardBody>
                </Card>

                <Card>
                  <CardHeader
                    title="系统状态"
                    actions={
                      <Link to="/settings" className="adm-link">
                        详情
                      </Link>
                    }
                  />
                  <CardBody>
                    <QueryView query={system}>
                      {({ components }) => <SystemStatusList components={components} />}
                    </QueryView>
                  </CardBody>
                </Card>
              </div>

              {items.length > 0 && (
                <div className="adm-section">
                  <div
                    className="oil-row oil-row--between"
                    style={{ marginBottom: "var(--oil-space-3)" }}
                  >
                    <h2 className="oil-title-md" style={{ margin: 0 }}>
                      最近的面试
                    </h2>
                    <Link to="/interviews" className="adm-link">
                      全部面试
                    </Link>
                  </div>
                  <Card>
                    <CardBody>
                      <ul className="adm-list">
                        {items.slice(0, 6).map((i) => (
                          <li key={i.id} className="adm-item adm-item--wrap">
                            <div className="adm-item__main">
                              <Person
                                name={i.candidate_name}
                                sub={`${i.position_title} · ${relativeTime(i.updated_at)}`}
                                to={`/interviews/${i.id}`}
                              />
                            </div>
                            <InterviewTags interview={i} />
                          </li>
                        ))}
                      </ul>
                    </CardBody>
                  </Card>
                </div>
              )}
            </>
          );
        }}
      </QueryView>
    </>
  );
}
