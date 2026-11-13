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
import { FileUp, UserPlus } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { errorMessage } from "../api/http";
import { useCandidates, useImportResume, useSaveCandidate } from "../api/queries";
import type { Candidate, CandidateInput } from "../api/types";
import { Empty, Pager, Person, QueryView, SearchInput } from "../components/common";
import { relativeTime } from "../lib/format";

const sourceLabel: Record<Candidate["source"], string> = {
  manual: "手动添加",
  resume: "上传简历",
  raas: "RAAS",
};

export const emptyCandidate: CandidateInput = {
  name: "",
  phone: null,
  email: null,
  current_title: null,
  current_company: null,
  years_of_experience: null,
  location: null,
  note: null,
};

/** 候选人基本信息的表单，新建和编辑共用 */
export function CandidateFields({
  value,
  onChange,
}: {
  value: CandidateInput;
  onChange: (next: CandidateInput) => void;
}) {
  const text = (key: keyof CandidateInput) => ({
    value: (value[key] as string | null) ?? "",
    onChange: (e: { target: { value: string } }) =>
      onChange({ ...value, [key]: e.target.value || (key === "name" ? "" : null) }),
  });
  return (
    <div className="adm-form-grid">
      <Field label="姓名" required>
        {({ id }) => <Input id={id} {...text("name")} autoFocus />}
      </Field>
      <Field label="所在城市">{({ id }) => <Input id={id} {...text("location")} />}</Field>
      <Field label="手机">{({ id }) => <Input id={id} type="tel" {...text("phone")} />}</Field>
      <Field label="邮箱">{({ id }) => <Input id={id} type="email" {...text("email")} />}</Field>
      <Field label="现在的职位">{({ id }) => <Input id={id} {...text("current_title")} />}</Field>
      <Field label="现在的公司">{({ id }) => <Input id={id} {...text("current_company")} />}</Field>
      <Field label="工作年限">
        {({ id }) => (
          <Input
            id={id}
            type="number"
            min={0}
            max={60}
            value={value.years_of_experience ?? ""}
            onChange={(e) =>
              onChange({
                ...value,
                years_of_experience:
                  e.target.value === "" ? null : Math.max(0, Number(e.target.value)),
              })
            }
          />
        )}
      </Field>
      <Field label="备注" className="adm-span">
        {({ id }) => <Textarea id={id} rows={3} {...text("note")} />}
      </Field>
    </div>
  );
}

function AddDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSaveCandidate();
  const [value, setValue] = useState(emptyCandidate);
  function submit(e: FormEvent) {
    e.preventDefault();
    save.mutate(value, {
      onSuccess: (c) => {
        setValue(emptyCandidate);
        onClose();
        navigate(`/candidates/${c.id}`);
      },
    });
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
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <CandidateFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
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
                    <th>面试</th>
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
                        {c.years_of_experience === null ? "—" : `${c.years_of_experience} 年`}
                      </td>
                      <td>
                        {c.profile_status === "pending" || c.profile_status === "running" ? (
                          <Badge tone="info">解析中</Badge>
                        ) : c.profile_status === "failed" ? (
                          <Badge tone="danger">解析失败</Badge>
                        ) : c.profile ? (
                          <span
                            className="oil-row oil-row--wrap"
                            style={{ "--oil-gap": "4px" } as React.CSSProperties}
                          >
                            {c.profile.skills.slice(0, 3).map((s) => (
                              <Badge key={s} plain>
                                {s}
                              </Badge>
                            ))}
                          </span>
                        ) : (
                          <span className="oil-muted">没有简历</span>
                        )}
                      </td>
                      <td className="oil-num">
                        {c.interview_count > 0 ? `${c.interview_count} 场` : "—"}
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
            accept=".pdf,.doc,.docx,.txt,.md,.png,.jpg,.jpeg"
            multiple
            title={importResume.isPending ? "正在上传…" : "把简历拖到这里，或者点一下选择文件"}
            hint="PDF、Word、图片都可以，一次可以传多份，每份不超过 20MB。"
            onFiles={(files) => void upload(files)}
          />
          <p className="oil-muted" style={{ margin: 0, fontSize: "var(--oil-text-sm)" }}>
            每份简历会建一位候选人。姓名、联系方式、经历和画像由大模型从简历里读出来，读完可以再改。
          </p>
        </div>
      </Dialog>
      <AddDialog open={adding} onClose={() => setAdding(false)} />
    </>
  );
}
