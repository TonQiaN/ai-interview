import {
  Badge,
  Banner,
  Button,
  Checkbox,
  Dialog,
  Field,
  Input,
  PageHeader,
  Segmented,
  Table,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { CloudDownload, Plus } from "lucide-react";
import { type FormEvent, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { errorMessage } from "../api/http";
import {
  useImportRaasPositions,
  usePositions,
  useRaasPositions,
  useRaasStatus,
  useSavePosition,
} from "../api/queries";
import type { PositionInput } from "../api/types";
import { Empty, Pager, QueryView, SearchInput } from "../components/common";
import { relativeTime } from "../lib/format";

export const emptyPosition: PositionInput = {
  title: "",
  department: null,
  location: null,
  employment_type: "全职",
  salary_range: null,
  headcount: null,
  description: "",
  requirements: "",
  status: "open",
};

/** 岗位的表单，新建和编辑共用 */
export function PositionFields({
  value,
  onChange,
}: {
  value: PositionInput;
  onChange: (next: PositionInput) => void;
}) {
  const text = (key: "department" | "location" | "employment_type" | "salary_range") => ({
    value: value[key] ?? "",
    onChange: (e: { target: { value: string } }) =>
      onChange({ ...value, [key]: e.target.value || null }),
  });
  return (
    <div className="adm-form-grid">
      <Field label="岗位名称" required className="adm-span">
        {({ id }) => (
          <Input
            id={id}
            value={value.title}
            autoFocus
            onChange={(e) => onChange({ ...value, title: e.target.value })}
          />
        )}
      </Field>
      <Field label="部门或客户">{({ id }) => <Input id={id} {...text("department")} />}</Field>
      <Field label="工作地点">{({ id }) => <Input id={id} {...text("location")} />}</Field>
      <Field label="用工形式">{({ id }) => <Input id={id} {...text("employment_type")} />}</Field>
      <Field label="薪资范围">
        {({ id }) => <Input id={id} placeholder="比如 25k–40k" {...text("salary_range")} />}
      </Field>
      <Field label="招几个人">
        {({ id }) => (
          <Input
            id={id}
            type="number"
            min={1}
            value={value.headcount ?? ""}
            onChange={(e) =>
              onChange({
                ...value,
                headcount: e.target.value === "" ? null : Math.max(1, Number(e.target.value)),
              })
            }
          />
        )}
      </Field>
      <Field label="状态">
        <Segmented
          aria-label="状态"
          value={value.status}
          options={[
            { value: "open", label: "在招" },
            { value: "closed", label: "已关闭" },
          ]}
          onChange={(status) => onChange({ ...value, status })}
        />
      </Field>
      <Field
        label="岗位职责"
        className="adm-span"
        hint="面试官会按这里和任职要求出题，写得越具体，题目越贴。"
      >
        {({ id, describedBy }) => (
          <Textarea
            id={id}
            aria-describedby={describedBy}
            rows={5}
            value={value.description}
            onChange={(e) => onChange({ ...value, description: e.target.value })}
          />
        )}
      </Field>
      <Field label="任职要求" className="adm-span">
        {({ id }) => (
          <Textarea
            id={id}
            rows={5}
            value={value.requirements}
            onChange={(e) => onChange({ ...value, requirements: e.target.value })}
          />
        )}
      </Field>
    </div>
  );
}

function AddDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSavePosition();
  const [value, setValue] = useState(emptyPosition);
  function submit(e: FormEvent) {
    e.preventDefault();
    save.mutate(value, {
      onSuccess: (p) => {
        setValue(emptyPosition);
        onClose();
        navigate(`/positions/${p.id}`);
      },
    });
  }
  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title="新建岗位"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            form="add-position"
            loading={save.isPending}
            disabled={!value.title.trim()}
          >
            新建
          </Button>
        </>
      }
    >
      <form id="add-position" onSubmit={submit} className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <PositionFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
}

function RaasDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useToast();
  const status = useRaasStatus();
  const [q, setQ] = useState("");
  const [page, setPage] = useState(1);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const available = status.data?.available === true;
  const list = useRaasPositions({ q, page, page_size: 8 }, open && available);
  const run = useImportRaasPositions();

  function toggle(ref: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(ref)) next.delete(ref);
      else next.add(ref);
      return next;
    });
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title="从 RAAS 导入岗位"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            disabled={picked.size === 0}
            loading={run.isPending}
            onClick={() =>
              run.mutate([...picked], {
                onSuccess: ({ imported }) => {
                  toast(`导入了 ${imported.length} 个岗位`);
                  setPicked(new Set());
                  onClose();
                },
                onError: (e) => toast(errorMessage(e)),
              })
            }
          >
            {picked.size > 0 ? `导入选中的 ${picked.size} 个` : "导入"}
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {status.data && !available && (
          <Banner tone="warning" title="现在连不上 RAAS">
            {status.data.message ?? "稍后再试，或者先手动新建岗位。"}
          </Banner>
        )}
        <p className="oil-muted" style={{ margin: 0, fontSize: "var(--oil-text-sm)" }}>
          只从 RAAS 读取，不会改动 RAAS 里的任何数据。导入之后是一份独立的副本，可以在这里再改。
        </p>
        <SearchInput
          value={q}
          placeholder="搜岗位、客户或地点"
          onChange={(value) => {
            setQ(value);
            setPage(1);
          }}
        />
        {available && (
          <QueryView query={list}>
            {(data) =>
              data.items.length === 0 ? (
                <p className="oil-muted">没有找到岗位。</p>
              ) : (
                <>
                  <ul className="adm-list">
                    {data.items.map((p) => (
                      <li key={p.ref} className="adm-item">
                        <div className="adm-item__main">
                          <Checkbox
                            checked={p.already_imported || picked.has(p.ref)}
                            disabled={p.already_imported}
                            onChange={() => toggle(p.ref)}
                            label={
                              <span>
                                <strong>{p.title}</strong>
                                <span className="adm-item__sub" style={{ display: "block" }}>
                                  {[
                                    p.client_name,
                                    p.location,
                                    p.headcount ? `招 ${p.headcount} 人` : null,
                                    p.ref,
                                  ]
                                    .filter(Boolean)
                                    .join(" · ")}
                                </span>
                              </span>
                            }
                          />
                        </div>
                        {p.already_imported ? (
                          <Badge plain>已导入</Badge>
                        ) : (
                          <span className="adm-item__sub">{relativeTime(p.updated_at)}更新</span>
                        )}
                      </li>
                    ))}
                  </ul>
                  <Pager
                    page={data.page}
                    pageSize={data.page_size}
                    total={data.total}
                    onPage={setPage}
                  />
                </>
              )
            }
          </QueryView>
        )}
      </div>
    </Dialog>
  );
}

export function PositionsPage() {
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const status = params.get("status") ?? "";
  const page = Number(params.get("page") ?? 1) || 1;
  const positions = usePositions({ q, status, page, page_size: 20 });
  const [adding, setAdding] = useState(false);
  const [importing, setImporting] = useState(false);

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
        title="岗位"
        description="面试官按岗位职责和任职要求出题。岗位可以自己写，也可以从 RAAS 导入。"
        actions={
          <>
            <Button
              variant="secondary"
              icon={<CloudDownload aria-hidden="true" />}
              onClick={() => setImporting(true)}
            >
              从 RAAS 导入
            </Button>
            <Button icon={<Plus aria-hidden="true" />} onClick={() => setAdding(true)}>
              新建岗位
            </Button>
          </>
        }
      />
      <div className="adm-toolbar">
        <SearchInput
          value={q}
          placeholder="搜岗位、部门或地点"
          onChange={(value) => update({ q: value, page: "" })}
        />
        <Segmented
          aria-label="按状态筛选"
          value={status === "open" || status === "closed" ? status : "all"}
          options={[
            { value: "all", label: "全部" },
            { value: "open", label: "在招" },
            { value: "closed", label: "已关闭" },
          ]}
          onChange={(value) => update({ status: value === "all" ? "" : value, page: "" })}
        />
      </div>
      <QueryView query={positions}>
        {(data) =>
          data.items.length === 0 ? (
            <Empty
              title={q || status ? "没有找到" : "还没有岗位"}
              description={q || status ? "换个条件试试。" : "新建一个岗位，或者从 RAAS 导入。"}
              actions={!q && !status && <Button onClick={() => setAdding(true)}>新建岗位</Button>}
            />
          ) : (
            <>
              <Table>
                <thead>
                  <tr>
                    <th>岗位</th>
                    <th>地点</th>
                    <th>薪资</th>
                    <th>人数</th>
                    <th>面试</th>
                    <th>来源</th>
                    <th>状态</th>
                    <th>更新</th>
                  </tr>
                </thead>
                <tbody>
                  {data.items.map((p) => (
                    <tr
                      key={p.id}
                      onClick={() => navigate(`/positions/${p.id}`)}
                      style={{ cursor: "pointer" }}
                    >
                      <td>
                        <Link to={`/positions/${p.id}`} className="adm-link">
                          {p.title}
                        </Link>
                        {p.department && <div className="adm-person__sub">{p.department}</div>}
                      </td>
                      <td>{p.location ?? "—"}</td>
                      <td>{p.salary_range ?? "—"}</td>
                      <td className="oil-num">{p.headcount ?? "—"}</td>
                      <td className="oil-num">
                        {p.interview_count > 0 ? `${p.interview_count} 场` : "—"}
                      </td>
                      <td className="oil-muted">{p.source === "raas" ? "RAAS" : "自己写的"}</td>
                      <td>
                        <Badge tone={p.status === "open" ? "success" : "neutral"}>
                          {p.status === "open" ? "在招" : "已关闭"}
                        </Badge>
                      </td>
                      <td className="oil-muted">{relativeTime(p.updated_at)}</td>
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
      <AddDialog open={adding} onClose={() => setAdding(false)} />
      <RaasDialog open={importing} onClose={() => setImporting(false)} />
    </>
  );
}
