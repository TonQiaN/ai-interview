import { Button, PageHeader, Select, Table } from "@ai-interview/ui";
import { Plus } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router";
import { useInterviews } from "../api/queries";
import type { InterviewStatus } from "../api/types";
import {
  DecisionBadge,
  Empty,
  Pager,
  Person,
  QueryView,
  SearchInput,
  StatusBadge,
} from "../components/common";
import { ReportVerdict } from "../components/ReportVerdict";
import { duration, elapsedMs, relativeTime, statusLabel } from "../lib/format";

const statuses = Object.keys(statusLabel) as InterviewStatus[];

export function InterviewsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const status = statuses.find((s) => s === params.get("status")) ?? "";
  const q = params.get("q") ?? "";
  const page = Number(params.get("page") ?? 1) || 1;
  // 默认新安排的在前；按分数排时没有报告的排在最后
  const byScore = params.get("sort") === "score";
  const interviews = useInterviews({
    status: status || undefined,
    q,
    page,
    page_size: 20,
    sort: byScore ? "-score" : undefined,
  });
  const filtered = Boolean(q || status);

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
      <div className="adm-toolbar">
        <SearchInput
          value={q}
          placeholder="搜候选人或岗位"
          onChange={(value) => update({ q: value, page: "" })}
        />
        <Select
          aria-label="按状态筛选"
          style={{ width: "auto" }}
          value={status}
          onChange={(e) => update({ status: e.target.value, page: "" })}
        >
          <option value="">全部状态</option>
          {statuses.map((s) => (
            <option key={s} value={s}>
              {statusLabel[s]}
            </option>
          ))}
        </Select>
        <Select
          aria-label="排序"
          style={{ width: "auto" }}
          value={byScore ? "score" : ""}
          onChange={(e) => update({ sort: e.target.value, page: "" })}
        >
          <option value="">新安排的在前</option>
          <option value="score">综合分高的在前</option>
        </Select>
      </div>
      <QueryView query={interviews}>
        {(data) =>
          data.items.length === 0 ? (
            <Empty
              title={filtered ? "没有找到" : "这里还没有面试"}
              description={filtered ? "换个关键词或者状态试试。" : "安排一场面试，它会出现在这里。"}
              actions={
                !filtered && <Button onClick={() => navigate("/interviews/new")}>安排面试</Button>
              }
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
                    <th>报告</th>
                    <th>决定</th>
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
                        <Person name={i.candidate_name} to={`/interviews/${i.id}`} />
                      </td>
                      <td>{i.position_title}</td>
                      <td>
                        <StatusBadge status={i.status} />
                      </td>
                      <td className="oil-num">{duration(elapsedMs(i))}</td>
                      <td>
                        <ReportVerdict interview={i} />
                      </td>
                      <td>
                        {i.decision === "pending" ? (
                          <span className="oil-muted">—</span>
                        ) : (
                          <DecisionBadge value={i.decision} />
                        )}
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
