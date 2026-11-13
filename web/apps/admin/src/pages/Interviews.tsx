import { Button, PageHeader, Table, Tabs } from "@ai-interview/ui";
import { Plus } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router";
import { useInterviews } from "../api/queries";
import {
  DecisionBadge,
  Empty,
  Pager,
  Person,
  QueryView,
  RecommendationBadge,
  SearchInput,
  StatusBadge,
} from "../components/common";
import { duration, relativeTime } from "../lib/format";

const groups = [
  { value: "all", label: "全部", status: "" },
  { value: "todo", label: "准备中", status: "draft,preparing,prepare_failed,ready" },
  { value: "invited", label: "已发链接", status: "invited,in_progress" },
  { value: "completed", label: "已完成", status: "completed" },
  { value: "closed", label: "没有完成", status: "abandoned,expired,cancelled" },
] as const;

type Group = (typeof groups)[number]["value"];

export function InterviewsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const group = (groups.find((g) => g.value === params.get("group")) ?? groups[0]).value as Group;
  const q = params.get("q") ?? "";
  const page = Number(params.get("page") ?? 1) || 1;
  const status = groups.find((g) => g.value === group)?.status ?? "";
  const interviews = useInterviews({ status, q, page, page_size: 20 });

  function update(patch: Record<string, string>) {
    const next = new URLSearchParams(params);
    for (const [k, v] of Object.entries(patch)) {
      if (v) next.set(k, v);
      else next.delete(k);
    }
    setParams(next, { replace: true });
  }

  return (
    <>
      <PageHeader
        title="面试"
        description="每一场面试从准备题目、发链接，到回看和报告，都在这里。"
        actions={
          <Button icon={<Plus aria-hidden="true" />} onClick={() => navigate("/interviews/new")}>
            安排面试
          </Button>
        }
      />
      <Tabs
        aria-label="按状态筛选"
        value={group}
        items={groups.map(({ value, label }) => ({ value, label }))}
        onChange={(value) => update({ group: value === "all" ? "" : value, page: "" })}
      />
      <div className="adm-toolbar" style={{ marginTop: "var(--oil-space-4)" }}>
        <SearchInput
          value={q}
          placeholder="搜候选人或岗位"
          onChange={(value) => update({ q: value, page: "" })}
        />
      </div>
      <QueryView query={interviews}>
        {(data) =>
          data.items.length === 0 ? (
            <Empty
              title={q ? "没有找到" : "这里还没有面试"}
              description={q ? "换个关键词试试。" : "安排一场面试，它会出现在这里。"}
              actions={!q && <Button onClick={() => navigate("/interviews/new")}>安排面试</Button>}
            />
          ) : (
            <>
              <Table>
                <thead>
                  <tr>
                    <th>候选人</th>
                    <th>岗位</th>
                    <th>状态</th>
                    <th>用时</th>
                    <th>得分</th>
                    <th>结论</th>
                    <th>更新</th>
                  </tr>
                </thead>
                <tbody>
                  {data.items.map((i) => (
                    <tr
                      key={i.id}
                      onClick={() => navigate(`/interviews/${i.id}`)}
                      style={{ cursor: "pointer" }}
                    >
                      <td>
                        <Person
                          name={i.candidate.name}
                          sub={i.candidate.current_title}
                          to={`/interviews/${i.id}`}
                        />
                      </td>
                      <td>
                        {i.position.title}
                        {i.position.department && (
                          <div className="adm-person__sub">{i.position.department}</div>
                        )}
                      </td>
                      <td>
                        <StatusBadge status={i.status} />
                      </td>
                      <td className="oil-num">
                        {i.duration_ms === null
                          ? `预计 ${i.expected_duration_min} 分钟`
                          : duration(i.duration_ms)}
                      </td>
                      <td className="oil-num">
                        {i.overall_score ?? (i.report_status === "running" ? "生成中" : "—")}
                      </td>
                      <td>
                        <span className="oil-row">
                          <RecommendationBadge value={i.recommendation} />
                          <DecisionBadge value={i.decision} />
                        </span>
                      </td>
                      <td className="oil-muted">{relativeTime(i.updated_at)}</td>
                    </tr>
                  ))}
                </tbody>
              </Table>
              <Pager
                page={data.page}
                pageSize={data.page_size}
                total={data.total}
                onPage={(p) => update({ page: p > 1 ? String(p) : "" })}
              />
            </>
          )
        }
      </QueryView>
    </>
  );
}
