import { Button, Card, CardBody, CardHeader, Hero, Stat } from "@ai-interview/ui";
import { ArrowRight, Plus } from "lucide-react";
import { Link, useNavigate } from "react-router";
import { useInterviews, useMe, useSystemStatus } from "../api/queries";
import type { InterviewSummary } from "../api/types";
import { DecisionBadge, Empty, Person, QueryView, StatusBadge } from "../components/common";
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

export function OverviewPage() {
  const navigate = useNavigate();
  const me = useMe();
  const interviews = useInterviews({ page_size: 100 });
  const system = useSystemStatus();

  return (
    <>
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
                {items.length === 0 ? (
                  <Empty title="还没有面试" description="安排第一场面试试试。" />
                ) : (
                  <Card>
                    <CardBody>
                      <ul className="adm-list">
                        {items.slice(0, 6).map((i) => (
                          <li key={i.id} className="adm-item">
                            <div className="adm-item__main">
                              <Person
                                name={i.candidate_name}
                                sub={`${i.position_title} · ${relativeTime(i.updated_at)}`}
                                to={`/interviews/${i.id}`}
                              />
                            </div>
                            <DecisionBadge value={i.decision} />
                            <StatusBadge status={i.status} />
                          </li>
                        ))}
                      </ul>
                    </CardBody>
                  </Card>
                )}
              </div>
            </>
          );
        }}
      </QueryView>
    </>
  );
}
