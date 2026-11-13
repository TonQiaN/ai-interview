import {
  Badge,
  Banner,
  Button,
  Dialog,
  Dropzone,
  Field,
  Input,
  PageHeader,
  Table,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { CloudDownload, FileUp, UserPlus } from "lucide-react";
import { type FormEvent, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { ApiError, errorMessage } from "../api/http";
import {
  profilePending,
  useCandidates,
  useImportRaasCandidates,
  useImportResume,
  useRaasCandidates,
  useSaveCandidate,
} from "../api/queries";
import type {
  CandidateInput,
  CandidateListItem,
  CandidateSource,
  DuplicateHint,
  RaasCandidate,
} from "../api/types";
import { Empty, Pager, Person, QueryView, SearchInput } from "../components/common";
import { RaasImportDialog } from "../components/RaasImportDialog";
import { relativeTime } from "../lib/format";

const sourceLabel: Record<CandidateSource, string> = {
  manual: "手动添加",
  upload: "上传简历",
  raas: "RAAS",
};

/** 上传简历能收的文件。Word 后端读不了，要先另存为 PDF */
export const RESUME_ACCEPT = ".pdf,.txt,.md,.png,.jpg,.jpeg,.webp";
export const RESUME_HINT = "PDF、图片、纯文本都可以，每份不超过 20MB。Word 文档请先另存为 PDF。";

/** 候选人基本信息表单里的值 */
export type CandidateForm = Required<Omit<CandidateInput, "allow_duplicate">>;

export const emptyCandidate: CandidateForm = {
  name: "",
  phone: null,
  email: null,
  current_title: null,
  current_company: null,
  location: null,
  notes: null,
};

/** 候选人基本信息的表单，新建和编辑共用 */
export function CandidateFields({
  value,
  onChange,
}: {
  value: CandidateForm;
  onChange: (next: CandidateForm) => void;
}) {
  const text = (key: keyof CandidateForm) => ({
    value: value[key] ?? "",
    onChange: (e: { target: { value: string } }) =>
      onChange({ ...value, [key]: e.target.value || (key === "name" ? "" : null) }),
  });
  return (
    <div className="adm-form-grid">
      <Field label="姓名" required>
        {({ id }) => <Input id={id} {...text("name")} />}
      </Field>
      <Field label="所在城市">{({ id }) => <Input id={id} {...text("location")} />}</Field>
      <Field label="手机">{({ id }) => <Input id={id} type="tel" {...text("phone")} />}</Field>
      <Field label="邮箱">{({ id }) => <Input id={id} type="email" {...text("email")} />}</Field>
      <Field label="现在的职位">{({ id }) => <Input id={id} {...text("current_title")} />}</Field>
      <Field label="现在的公司">{({ id }) => <Input id={id} {...text("current_company")} />}</Field>
      <Field label="备注" className="adm-span">
        {({ id }) => <Textarea id={id} rows={3} {...text("notes")} />}
      </Field>
    </div>
  );
}

/** 接口说「已经有这个人了」时，带回来的那些候选人 */
function duplicatesOf(error: unknown): DuplicateHint[] | null {
  if (!(error instanceof ApiError) || error.code !== "duplicate_candidate") return null;
  const list = (error.details as { duplicates?: DuplicateHint[] } | null)?.duplicates;
  return Array.isArray(list) ? list : [];
}

/** 可能是同一个人的候选人，一人一行，点了去看 */
export function DuplicateList({ items }: { items: DuplicateHint[] }) {
  return (
    <ul className="adm-plain-list">
      {items.map((d) => (
        <li key={d.id}>
          <Link to={`/candidates/${d.id}`} className="adm-link">
            {d.name}
          </Link>
          <span className="oil-muted">
            {" "}
            {[d.current_title, d.current_company, `${relativeTime(d.created_at)}添加`]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </li>
      ))}
    </ul>
  );
}

function AddDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSaveCandidate();
  const [value, setValue] = useState(emptyCandidate);
  const duplicates = duplicatesOf(save.error);

  function create(allowDuplicate: boolean) {
    save.mutate(
      { ...value, allow_duplicate: allowDuplicate },
      {
        onSuccess: (c) => {
          setValue(emptyCandidate);
          onClose();
          navigate(`/candidates/${c.id}`);
        },
      },
    );
  }
  function submit(e: FormEvent) {
    e.preventDefault();
    create(false);
  }
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="手动添加候选人"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            form="add-candidate"
            loading={save.isPending}
            disabled={!value.name.trim()}
          >
            添加
          </Button>
        </>
      }
    >
      <form id="add-candidate" onSubmit={submit} className="oil-stack">
        {duplicates ? (
          <Banner
            tone="warning"
            title="手机号或邮箱和已有的候选人一样"
            actions={
              <Button size="sm" variant="secondary" onClick={() => create(true)}>
                不是同一个人，照样添加
              </Button>
            }
          >
            <DuplicateList items={duplicates} />
          </Banner>
        ) : (
          save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>
        )}
        <CandidateFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
}

function RaasDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const run = useImportRaasCandidates();
  return (
    <RaasImportDialog
      open={open}
      onClose={onClose}
      title="从 RAAS 导入候选人"
      unit="位候选人"
      searchPlaceholder="搜姓名、公司或职位"
      fallback="现在可以先上传简历。"
      list={useRaasCandidates}
      run={run}
      itemName={(item) => item.name ?? item.external_ref}
      doneMessage={(items) => {
        const reading = items.filter((i) => i.resume === "queued").length;
        return reading > 0
          ? `导入了 ${items.length} 位候选人，${reading} 份简历正在读`
          : `导入了 ${items.length} 位候选人`;
      }}
      row={(c: RaasCandidate) => ({
        ref: c.external_ref,
        title: c.name ?? "没有姓名",
        sub: [
          c.current_title,
          c.current_company,
          c.current_city,
          c.degree,
          c.experience_years == null ? null : `${c.experience_years} 年`,
          c.latest_application_job_title ? `投了 ${c.latest_application_job_title}` : null,
        ],
        imported: Boolean(c.imported_candidate_id),
      })}
    />
  );
}

/** 列表里「画像」那一格：简历读到哪一步了，读完显示几个核心技能 */
function ProfileCell({ candidate: c }: { candidate: CandidateListItem }) {
  const status = c.primary_resume?.parse_status;
  if (status === "pending" || status === "running") return <Badge tone="info">正在读简历</Badge>;
  if (status === "failed") return <Badge tone="danger">简历没读出来</Badge>;
  if (profilePending(c)) return <Badge tone="info">正在生成画像</Badge>;
  if (c.profile.core_skills.length > 0) {
    return (
      <span className="oil-row oil-row--wrap" style={{ "--oil-gap": "4px" } as React.CSSProperties}>
        {c.profile.core_skills.slice(0, 3).map((s) => (
          <Badge key={s.name} plain>
            {s.name}
          </Badge>
        ))}
      </span>
    );
  }
  if (c.profile.headline) return <span>{c.profile.headline}</span>;
  return <span className="oil-muted">{c.primary_resume ? "还没有画像" : "没有简历"}</span>;
}

export function CandidatesPage() {
  const navigate = useNavigate();
  const toast = useToast();
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const page = Number(params.get("page") ?? 1) || 1;
  const candidates = useCandidates({ q, page, page_size: 20 });
  const importResume = useImportResume();
  const [adding, setAdding] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [importing, setImporting] = useState(false);

  async function upload(files: File[]) {
    let ok = 0;
    for (const file of files) {
      try {
        await importResume.mutateAsync(file);
        ok += 1;
      } catch (e) {
        toast(`${file.name}：${errorMessage(e)}`);
      }
    }
    if (ok > 0) {
      toast(`收到 ${ok} 份简历，正在解析`);
      setUploading(false);
    }
  }

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
        title="候选人"
        description="上传简历就行，姓名、经历和画像由大模型读出来。"
        actions={
          <>
            <Button
              variant="secondary"
              icon={<CloudDownload aria-hidden="true" />}
              onClick={() => setImporting(true)}
            >
              从 RAAS 导入
            </Button>
            <Button
              variant="secondary"
              icon={<UserPlus aria-hidden="true" />}
              onClick={() => setAdding(true)}
            >
              手动添加
            </Button>
            <Button icon={<FileUp aria-hidden="true" />} onClick={() => setUploading(true)}>
              上传简历
            </Button>
          </>
        }
      />
      <div className="adm-toolbar">
        <SearchInput
          value={q}
          placeholder="搜姓名、公司、职位、手机或邮箱"
          onChange={(value) => update({ q: value, page: "" })}
        />
      </div>
      <QueryView query={candidates}>
        {(data) =>
          data.items.length === 0 ? (
            <Empty
              title={q ? "没有找到" : "还没有候选人"}
              description={q ? "换个关键词试试。" : "上传一份简历，候选人就建好了。"}
              actions={!q && <Button onClick={() => setUploading(true)}>上传简历</Button>}
            />
          ) : (
            <>
              <Table>
                <thead>
                  <tr>
                    <th>候选人</th>
                    <th>现在</th>
                    <th>年限</th>
                    <th>画像</th>
                    <th>来源</th>
                    <th>更新</th>
                  </tr>
                </thead>
                <tbody>
                  {data.items.map((c) => (
                    <tr
                      key={c.id}
                      onClick={() => navigate(`/candidates/${c.id}`)}
                      style={{ cursor: "pointer" }}
                    >
                      <td>
                        <Person name={c.name} sub={c.location} to={`/candidates/${c.id}`} />
                      </td>
                      <td>
                        {c.current_title ?? "—"}
                        {c.current_company && (
                          <div className="adm-person__sub">{c.current_company}</div>
                        )}
                      </td>
                      <td className="oil-num">
                        {c.profile.years_of_experience == null
                          ? "—"
                          : `${c.profile.years_of_experience} 年`}
                      </td>
                      <td>
                        <ProfileCell candidate={c} />
                      </td>
                      <td className="oil-muted">{sourceLabel[c.source]}</td>
                      <td className="oil-muted">{relativeTime(c.updated_at)}</td>
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

      <Dialog open={uploading} onClose={() => setUploading(false)} title="上传简历">
        <div className="oil-stack">
          <Dropzone
            accept={RESUME_ACCEPT}
            multiple
            title={importResume.isPending ? "正在上传…" : "把简历拖到这里，或者点一下选择文件"}
            hint={`一次可以传多份。${RESUME_HINT}`}
            onFiles={(files) => void upload(files)}
          />
          <p className="oil-muted" style={{ margin: 0, fontSize: "var(--oil-text-sm)" }}>
            每份简历会建一位候选人。姓名、联系方式、经历和画像由大模型从简历里读出来，读完可以再改。
          </p>
        </div>
      </Dialog>
      <AddDialog open={adding} onClose={() => setAdding(false)} />
      <RaasDialog open={importing} onClose={() => setImporting(false)} />
    </>
  );
}
