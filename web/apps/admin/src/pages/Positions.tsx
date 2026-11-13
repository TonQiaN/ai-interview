import {
  Badge,
  Banner,
  Button,
  Dialog,
  Field,
  Input,
  PageHeader,
  Segmented,
  Table,
  Textarea,
} from "@ai-interview/ui";
import { CloudDownload, Plus, Sparkles } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { errorMessage } from "../api/http";
import {
  useDraftPosition,
  useImportRaasPositions,
  usePositions,
  useRaasPositions,
  useSavePosition,
} from "../api/queries";
import type {
  PositionDraft,
  PositionExtra,
  PositionInput,
  PositionStatus,
  RaasPosition,
} from "../api/types";
import { Empty, Pager, QueryView, SearchInput } from "../components/common";
import { RaasImportDialog } from "../components/RaasImportDialog";
import { positionStatusLabel, positionStatusTone, relativeTime } from "../lib/format";

export const emptyExtra: PositionExtra = {
  benefits: null,
  client_name: null,
  degree_requirement: null,
  evaluation_rules: null,
  experience_requirement: null,
  hard_requirements: [],
  headcount: null,
  interview_focus: [],
  keywords: [],
  language_requirement: null,
  must_have_skills: [],
  negative_requirements: [],
  nice_to_have: [],
  nice_to_have_skills: [],
  raas: null,
  salary_range: null,
};

/** 岗位表单里的值。比接口的请求体严格一点：extra 和状态一定有 */
export type PositionForm = PositionInput & { extra: PositionExtra; status: PositionStatus };

export const emptyPosition: PositionForm = {
  title: "",
  department: null,
  location: null,
  employment_type: "全职",
  seniority: null,
  description: "",
  requirements: "",
  status: "open",
  extra: emptyExtra,
};

/** 一行一条的文字和数组互相转 */
const toLines = (items: string[]) => items.join("\n");
const fromLines = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

/** 岗位的表单，新建和编辑共用 */
export function PositionFields({
  value,
  onChange,
}: {
  value: PositionForm;
  onChange: (next: PositionForm) => void;
}) {
  const text = (key: "department" | "location" | "employment_type" | "seniority") => ({
    value: value[key] ?? "",
    onChange: (e: { target: { value: string } }) =>
      onChange({ ...value, [key]: e.target.value || null }),
  });
  const extraText = (key: "salary_range" | "headcount" | "client_name") => ({
    value: value.extra[key] ?? "",
    onChange: (e: { target: { value: string } }) =>
      onChange({ ...value, extra: { ...value.extra, [key]: e.target.value || null } }),
  });
  // 一行一条的输入框：打字的时候先留着原样（包括空行），失焦再整理
  const [focusText, setFocusText] = useState(() => toLines(value.extra.interview_focus));
  const [hardText, setHardText] = useState(() => toLines(value.extra.hard_requirements));
  const focusKey = toLines(value.extra.interview_focus);
  const hardKey = toLines(value.extra.hard_requirements);
  // biome-ignore lint/correctness/useExhaustiveDependencies: 外面换了一份数据（比如自动整理填进来）才重置
  useEffect(() => {
    if (toLines(fromLines(focusText)) !== focusKey) setFocusText(focusKey);
    if (toLines(fromLines(hardText)) !== hardKey) setHardText(hardKey);
  }, [focusKey, hardKey]);

  return (
    <div className="adm-form-grid">
      <Field label="岗位名称" required className="adm-span">
        {({ id }) => (
          <Input
            id={id}
            value={value.title}
            onChange={(e) => onChange({ ...value, title: e.target.value })}
          />
        )}
      </Field>
      <Field label="部门">{({ id }) => <Input id={id} {...text("department")} />}</Field>
      <Field label="用人方" hint="外包岗位写客户公司的名字。">
        {({ id, describedBy }) => (
          <Input id={id} aria-describedby={describedBy} {...extraText("client_name")} />
        )}
      </Field>
      <Field label="工作地点">{({ id }) => <Input id={id} {...text("location")} />}</Field>
      <Field label="用工形式">{({ id }) => <Input id={id} {...text("employment_type")} />}</Field>
      <Field label="职级">
        {({ id }) => <Input id={id} placeholder="比如 高级、P6" {...text("seniority")} />}
      </Field>
      <Field label="薪资范围">
        {({ id }) => <Input id={id} placeholder="比如 25k–40k" {...extraText("salary_range")} />}
      </Field>
      <Field label="招几个人">
        {({ id }) => <Input id={id} placeholder="比如 2 人" {...extraText("headcount")} />}
      </Field>
      <Field label="状态">
        <Segmented
          aria-label="状态"
          value={value.status}
          options={[
            { value: "open", label: "在招" },
            { value: "paused", label: "暂停" },
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
            value={value.description ?? ""}
            onChange={(e) => onChange({ ...value, description: e.target.value })}
          />
        )}
      </Field>
      <Field label="任职要求" className="adm-span">
        {({ id }) => (
          <Textarea
            id={id}
            rows={5}
            value={value.requirements ?? ""}
            onChange={(e) => onChange({ ...value, requirements: e.target.value })}
          />
        )}
      </Field>
      <Field label="硬性要求" hint="不满足就不合适的条件，一行一条。">
        {({ id, describedBy }) => (
          <Textarea
            id={id}
            aria-describedby={describedBy}
            rows={4}
            value={hardText}
            onChange={(e) => {
              setHardText(e.target.value);
              onChange({
                ...value,
                extra: { ...value.extra, hard_requirements: fromLines(e.target.value) },
              });
            }}
          />
        )}
      </Field>
      <Field label="面试重点考察" hint="出题时会照顾到，一行一条。">
        {({ id, describedBy }) => (
          <Textarea
            id={id}
            aria-describedby={describedBy}
            rows={4}
            value={focusText}
            onChange={(e) => {
              setFocusText(e.target.value);
              onChange({
                ...value,
                extra: { ...value.extra, interview_focus: fromLines(e.target.value) },
              });
            }}
          />
        )}
      </Field>
    </div>
  );
}

/** 粘贴一段岗位描述，让大模型整理成表单 */
function DraftBox({ onDraft }: { onDraft: (draft: PositionDraft) => void }) {
  const draft = useDraftPosition();
  const [text, setText] = useState("");
  return (
    <div className="adm-draft">
      <Field
        label="有现成的岗位描述？"
        hint="把招聘网站或者文档里的整段文字贴进来，自动拆成下面的表单，拆完还可以再改。"
      >
        {({ id, describedBy }) => (
          <Textarea
            id={id}
            aria-describedby={describedBy}
            rows={3}
            value={text}
            placeholder="粘贴岗位描述"
            onChange={(e) => setText(e.target.value)}
          />
        )}
      </Field>
      {draft.error && <Banner tone="danger">{errorMessage(draft.error)}</Banner>}
      <div>
        <Button
          variant="secondary"
          size="sm"
          icon={<Sparkles aria-hidden="true" />}
          loading={draft.isPending}
          disabled={!text.trim()}
          onClick={() => draft.mutate(text, { onSuccess: onDraft })}
        >
          {draft.isPending ? "正在整理" : "自动整理"}
        </Button>
      </div>
    </div>
  );
}

function AddDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSavePosition();
  const [value, setValue] = useState<PositionForm>(emptyPosition);
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
        <DraftBox
          onDraft={(draft) =>
            setValue({ ...draft, status: "open", extra: { ...emptyExtra, ...draft.extra } })
          }
        />
        <PositionFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
}

function RaasDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const run = useImportRaasPositions();
  return (
    <RaasImportDialog
      open={open}
      onClose={onClose}
      title="从 RAAS 导入岗位"
      unit="个岗位"
      searchPlaceholder="搜岗位名称"
      fallback="现在可以先手动新建岗位。"
      list={useRaasPositions}
      run={run}
      itemName={(item) => item.title ?? item.external_ref}
      row={(p: RaasPosition) => ({
        ref: p.external_ref,
        title: p.title ?? "没有名称",
        sub: [
          p.client_name,
          p.cities.join("、"),
          p.salary_range,
          p.headcount ? `招 ${p.headcount} 人` : null,
          p.status === "open" ? null : positionStatusLabel[p.status],
          p.external_ref,
        ],
        imported: Boolean(p.imported_position_id),
        aside: p.updated_at && (
          <span className="adm-item__sub">{relativeTime(p.updated_at)}更新</span>
        ),
      })}
    />
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
          value={status === "open" || status === "paused" || status === "closed" ? status : "all"}
          options={[
            { value: "all", label: "全部" },
            { value: "open", label: "在招" },
            { value: "paused", label: "暂停" },
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
                        {(p.department || p.extra.client_name) && (
                          <div className="adm-person__sub">
                            {[p.extra.client_name, p.department].filter(Boolean).join(" · ")}
                          </div>
                        )}
                      </td>
                      <td>{p.location ?? "—"}</td>
                      <td>{p.extra.salary_range ?? "—"}</td>
                      <td>{p.extra.headcount ?? "—"}</td>
                      <td className="oil-muted">{p.source === "raas" ? "RAAS" : "自己写的"}</td>
                      <td>
                        <Badge tone={positionStatusTone[p.status]}>
                          {positionStatusLabel[p.status]}
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
