import {
  Avatar,
  Badge,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  Dialog,
  Dropzone,
  KeyValue,
  PageHeader,
  useToast,
} from "@ai-interview/ui";
import { FileText, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { errorMessage } from "../api/http";
import {
  useCandidate,
  useDeleteCandidate,
  useInterviews,
  useRegenerateProfile,
  useResumes,
  useSaveCandidate,
  useUploadResume,
} from "../api/queries";
import type { Candidate, CandidateInput, CandidateProfile } from "../api/types";
import { Confirm, QueryView, RecommendationBadge, StatusBadge } from "../components/common";
import {
  fileSize,
  parseStatusLabel,
  parseStatusTone,
  relativeTime,
  yearMonth,
} from "../lib/format";
import { CandidateFields } from "./Candidates";

function Profile({ profile }: { profile: CandidateProfile }) {
  return (
    <div className="oil-stack" style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}>
      <p style={{ margin: 0, fontSize: "var(--oil-text-lg)" }}>{profile.summary}</p>
      {profile.skills.length > 0 && (
        <div className="adm-chips">
          {profile.skills.map((s) => (
            <Chip key={s}>{s}</Chip>
          ))}
        </div>
      )}
      {profile.highlights.length > 0 && (
        <section>
          <h3 className="oil-title-sm">亮点</h3>
          <ul className="adm-bullets">
            {profile.highlights.map((h) => (
              <li key={h}>{h}</li>
            ))}
          </ul>
        </section>
      )}
      {profile.to_verify.length > 0 && (
        <section>
          <h3 className="oil-title-sm">面试时值得核实</h3>
          <ul className="adm-bullets">
            {profile.to_verify.map((h) => (
              <li key={h}>{h}</li>
            ))}
          </ul>
        </section>
      )}
      {profile.experiences.length > 0 && (
        <section>
          <h3 className="oil-title-sm">工作经历</h3>
          <ul className="adm-list">
            {profile.experiences.map((e) => (
              <li
                key={`${e.company}-${e.start}`}
                className="adm-item"
                style={{ alignItems: "flex-start" }}
              >
                <div className="adm-item__main">
                  <strong>
                    {e.title} · {e.company}
                  </strong>
                  <div className="adm-item__sub">
                    {yearMonth(e.start)} – {yearMonth(e.end)}
                  </div>
                  <p style={{ margin: "var(--oil-space-2) 0 0" }}>{e.summary}</p>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {profile.education.length > 0 && (
        <section>
          <h3 className="oil-title-sm">教育</h3>
          <ul className="adm-list">
            {profile.education.map((e) => (
              <li key={`${e.school}-${e.start}`} className="adm-item">
                <div className="adm-item__main">
                  <strong>{e.school}</strong>
                  <div className="adm-item__sub">
                    {[e.degree, e.major].filter(Boolean).join(" · ")}
                    {e.start || e.end ? `，${yearMonth(e.start)} – ${yearMonth(e.end)}` : ""}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function EditDialog({
  candidate,
  open,
  onClose,
}: {
  candidate: Candidate;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSaveCandidate(candidate.id);
  const toast = useToast();
  const pick = (c: Candidate): CandidateInput => ({
    name: c.name,
    phone: c.phone,
    email: c.email,
    current_title: c.current_title,
    current_company: c.current_company,
    years_of_experience: c.years_of_experience,
    location: c.location,
    note: c.note,
  });
  const [value, setValue] = useState(() => pick(candidate));
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开时把表单重置成最新的数据
  useEffect(() => {
    if (open) setValue(pick(candidate));
  }, [open]);
  function submit(e: FormEvent) {
    e.preventDefault();
    save.mutate(value, {
      onSuccess: () => {
        toast("已保存");
        onClose();
      },
    });
  }
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="改基本信息"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            form="edit-candidate"
            loading={save.isPending}
            disabled={!value.name.trim()}
          >
            保存
          </Button>
        </>
      }
    >
      <form id="edit-candidate" onSubmit={submit} className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <CandidateFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
}

function Detail({ candidate }: { candidate: Candidate }) {
  const navigate = useNavigate();
  const toast = useToast();
  const resumes = useResumes(candidate.id);
  const interviews = useInterviews({ candidate_id: candidate.id, page_size: 50 });
  const upload = useUploadResume(candidate.id);
  const regenerate = useRegenerateProfile(candidate.id);
  const remove = useDeleteCandidate();
  const [editing, setEditing] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const parsing = candidate.profile_status === "pending" || candidate.profile_status === "running";

  return (
    <>
      <PageHeader
        crumbs={<Link to="/candidates">候选人</Link>}
        title={candidate.name}
        description={
          [candidate.current_title, candidate.current_company, candidate.location]
            .filter(Boolean)
            .join(" · ") || undefined
        }
        actions={
          <>
            <Button
              variant="secondary"
              icon={<Pencil aria-hidden="true" />}
              onClick={() => setEditing(true)}
            >
              改信息
            </Button>
            <Button
              icon={<Plus aria-hidden="true" />}
              onClick={() => navigate(`/interviews/new?candidate=${candidate.id}`)}
            >
              安排面试
            </Button>
          </>
        }
      />
      <div className="adm-split">
        <Card>
          <CardHeader
            title="画像"
            actions={
              candidate.profile && (
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<RefreshCw aria-hidden="true" />}
                  loading={regenerate.isPending}
                  onClick={() =>
                    regenerate.mutate(undefined, {
                      onSuccess: () => toast("正在重新读简历"),
                      onError: (e) => toast(errorMessage(e)),
                    })
                  }
                >
                  重新生成
                </Button>
              )
            }
          />
          <CardBody>
            {parsing ? (
              <Banner tone="info" title="正在读简历">
                大模型在读这份简历，一般几十秒。读完这里会自动出现画像。
              </Banner>
            ) : candidate.profile_status === "failed" ? (
              <Banner
                tone="danger"
                title="简历没有读出来"
                actions={
                  <Button
                    size="sm"
                    loading={regenerate.isPending}
                    onClick={() =>
                      regenerate.mutate(undefined, { onError: (e) => toast(errorMessage(e)) })
                    }
                  >
                    再读一次
                  </Button>
                }
              >
                可能是文件损坏或者是扫描件看不清。可以重试，或者换一份文件上传。
              </Banner>
            ) : candidate.profile ? (
              <Profile profile={candidate.profile} />
            ) : (
              <div className="oil-stack">
                <p className="oil-muted" style={{ margin: 0 }}>
                  还没有画像。上传一份简历，大模型会读出经历、技能和值得核实的地方，面试官出题时也会用到。
                </p>
                <div>
                  <Button variant="secondary" onClick={() => setUploading(true)}>
                    上传简历
                  </Button>
                </div>
              </div>
            )}
          </CardBody>
        </Card>

        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <Card>
            <CardBody>
              <div className="oil-row" style={{ marginBottom: "var(--oil-space-4)" }}>
                <Avatar name={candidate.name} size="lg" />
                <div>
                  <strong>{candidate.name}</strong>
                  <div className="adm-item__sub">
                    {candidate.years_of_experience === null
                      ? "工作年限没填"
                      : `工作 ${candidate.years_of_experience} 年`}
                  </div>
                </div>
              </div>
              <KeyValue
                items={[
                  ["手机", candidate.phone ?? "—"],
                  ["邮箱", candidate.email ?? "—"],
                  ["城市", candidate.location ?? "—"],
                  [
                    "备注",
                    candidate.note ? (
                      <span key="note" className="adm-prewrap">
                        {candidate.note}
                      </span>
                    ) : (
                      "—"
                    ),
                  ],
                ]}
              />
            </CardBody>
          </Card>

          <Card>
            <CardHeader
              title="简历"
              actions={
                <Button size="sm" variant="secondary" onClick={() => setUploading(true)}>
                  上传
                </Button>
              }
            />
            <CardBody>
              <QueryView query={resumes}>
                {({ items }) =>
                  items.length === 0 ? (
                    <p className="oil-muted" style={{ margin: 0 }}>
                      还没有简历。
                    </p>
                  ) : (
                    <ul className="adm-list">
                      {items.map((r) => (
                        <li key={r.id} className="adm-item">
                          <FileText aria-hidden="true" size={18} />
                          <div className="adm-item__main">
                            <div className="adm-ellipsis">{r.file_name}</div>
                            <div className="adm-item__sub">
                              {fileSize(r.size_bytes)} · {relativeTime(r.created_at)}
                              {r.parse_error ? ` · ${r.parse_error}` : ""}
                            </div>
                          </div>
                          <Badge tone={parseStatusTone[r.parse_status]} plain>
                            {parseStatusLabel[r.parse_status]}
                          </Badge>
                        </li>
                      ))}
                    </ul>
                  )
                }
              </QueryView>
            </CardBody>
          </Card>

          <Card>
            <CardHeader title="面试" />
            <CardBody>
              <QueryView query={interviews}>
                {({ items }) =>
                  items.length === 0 ? (
                    <p className="oil-muted" style={{ margin: 0 }}>
                      还没有安排过面试。
                    </p>
                  ) : (
                    <ul className="adm-list">
                      {items.map((i) => (
                        <li key={i.id} className="adm-item">
                          <div className="adm-item__main">
                            <Link to={`/interviews/${i.id}`} className="adm-link">
                              {i.position.title}
                            </Link>
                            <div className="adm-item__sub">{relativeTime(i.updated_at)}</div>
                          </div>
                          {i.overall_score !== null && (
                            <strong className="oil-num">{i.overall_score}</strong>
                          )}
                          <RecommendationBadge value={i.recommendation} />
                          <StatusBadge status={i.status} />
                        </li>
                      ))}
                    </ul>
                  )
                }
              </QueryView>
            </CardBody>
          </Card>

          <div>
            <Button
              variant="ghost"
              size="sm"
              icon={<Trash2 aria-hidden="true" />}
              onClick={() => setDeleting(true)}
            >
              删除候选人
            </Button>
          </div>
        </div>
      </div>

      <EditDialog candidate={candidate} open={editing} onClose={() => setEditing(false)} />
      <Dialog
        open={uploading}
        onClose={() => setUploading(false)}
        title={`给${candidate.name}上传简历`}
      >
        <Dropzone
          accept=".pdf,.doc,.docx,.txt,.md,.png,.jpg,.jpeg"
          title={upload.isPending ? "正在上传…" : "把简历拖到这里，或者点一下选择文件"}
          hint="新简历读完后，画像会按它更新。"
          onFiles={([file]) => {
            if (!file) return;
            upload.mutate(file, {
              onSuccess: () => {
                toast("收到了，正在读简历");
                setUploading(false);
              },
              onError: (e) => toast(errorMessage(e)),
            });
          }}
        />
      </Dialog>
      <Confirm
        open={deleting}
        title={`删除${candidate.name}？`}
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setDeleting(false)}
        onConfirm={() =>
          remove.mutate(candidate.id, {
            onSuccess: () => {
              toast("已删除");
              navigate("/candidates", { replace: true });
            },
            onError: (e) => {
              toast(errorMessage(e));
              setDeleting(false);
            },
          })
        }
      >
        简历和画像会一起删掉，找不回来。有面试记录的候选人不能删。
      </Confirm>
    </>
  );
}

export function CandidateDetailPage() {
  const { id } = useParams();
  const candidate = useCandidate(id);
  return <QueryView query={candidate}>{(c) => <Detail candidate={c} />}</QueryView>;
}
