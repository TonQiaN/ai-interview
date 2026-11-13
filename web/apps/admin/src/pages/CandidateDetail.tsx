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
import { errorMessage, isMock } from "../api/http";
import {
  profilePending,
  useCandidate,
  useDeleteCandidate,
  useDeleteResume,
  useInterviews,
  useRegenerateProfile,
  useReparseResume,
  useResume,
  useResumes,
  useSaveCandidate,
  useUploadResume,
} from "../api/queries";
import type {
  Candidate,
  CandidateDetail,
  CandidateProfile,
  ParsedResume,
  Resume,
} from "../api/types";
import { Confirm, QueryView, RecommendationBadge, StatusBadge } from "../components/common";
import {
  fileSize,
  parseStatusLabel,
  parseStatusTone,
  relativeTime,
  yearMonth,
} from "../lib/format";
import {
  CandidateFields,
  type CandidateForm,
  DuplicateList,
  RESUME_ACCEPT,
  RESUME_HINT,
} from "./Candidates";

/** 起止时间。在职的写「至今」 */
function period(start: string | null, end: string | null, isCurrent = false): string {
  if (!start && !end && !isCurrent) return "";
  return `${yearMonth(start)} – ${isCurrent && !end ? "至今" : yearMonth(end)}`;
}

function Bullets({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <section>
      <h3 className="oil-title-sm">{title}</h3>
      <ul className="adm-bullets">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </section>
  );
}

/** 画像：大模型读完简历之后的归纳，不是简历原文 */
function Profile({ profile }: { profile: CandidateProfile }) {
  const facts = [
    profile.seniority,
    profile.years_of_experience == null ? null : `工作 ${profile.years_of_experience} 年`,
    profile.highest_degree,
    ...profile.domains,
  ].filter((v): v is string => Boolean(v));
  return (
    <div className="oil-stack" style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}>
      {profile.headline && (
        <p style={{ margin: 0, fontSize: "var(--oil-text-lg)" }}>{profile.headline}</p>
      )}
      {profile.summary && <p style={{ margin: 0 }}>{profile.summary}</p>}
      {facts.length > 0 && (
        <div className="adm-chips">
          {facts.map((f) => (
            <Chip key={f}>{f}</Chip>
          ))}
        </div>
      )}
      {profile.core_skills.length > 0 && (
        <section>
          <h3 className="oil-title-sm">核心技能</h3>
          {profile.core_skills.some((skill) => skill.evidence) ? (
            <ul className="adm-list">
              {profile.core_skills.map((skill) => (
                <li key={skill.name} className="adm-item" style={{ alignItems: "flex-start" }}>
                  <div className="adm-item__main">
                    <strong>{skill.name}</strong>
                    {skill.evidence && <div className="adm-item__sub">{skill.evidence}</div>}
                  </div>
                  {skill.level && <Badge plain>{skill.level}</Badge>}
                </li>
              ))}
            </ul>
          ) : (
            // 没有依据可写的时候不占一人一行，排成一排
            <div className="adm-chips">
              {profile.core_skills.map((skill) => (
                <Chip key={skill.name}>
                  {skill.name}
                  {skill.level ? `（${skill.level}）` : ""}
                </Chip>
              ))}
            </div>
          )}
        </section>
      )}
      <Bullets title="亮点" items={profile.strengths} />
      <Bullets title="疑点" items={profile.risks} />
      <Bullets title="面试时值得核实" items={profile.interview_focus} />
    </div>
  );
}

/** 简历里读出来的经历、项目和教育 */
function ResumeContent({ parsed }: { parsed: ParsedResume }) {
  const intention = [
    parsed.job_intention.positions.join("、"),
    parsed.job_intention.cities.join("、"),
    parsed.job_intention.salary,
    parsed.job_intention.availability,
  ].filter(Boolean);
  const empty =
    parsed.work_experience.length === 0 &&
    parsed.projects.length === 0 &&
    parsed.education.length === 0 &&
    parsed.skills.length === 0;
  if (empty) {
    return (
      <p className="oil-muted" style={{ margin: 0 }}>
        这份简历里没有读出经历。
      </p>
    );
  }
  return (
    <div className="oil-stack" style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}>
      {parsed.work_experience.length > 0 && (
        <section>
          <h3 className="oil-title-sm">工作经历</h3>
          <ul className="adm-list">
            {parsed.work_experience.map((e, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: 经历没有 id，公司和时间可能都是空的
                key={index}
                className="adm-item"
                style={{ alignItems: "flex-start" }}
              >
                <div className="adm-item__main">
                  <strong>{[e.title, e.company].filter(Boolean).join(" · ") || "没写职位"}</strong>
                  <div className="adm-item__sub">
                    {[period(e.start, e.end, e.is_current), e.location].filter(Boolean).join(" · ")}
                  </div>
                  {e.description && (
                    <p className="adm-prewrap" style={{ margin: "var(--oil-space-2) 0 0" }}>
                      {e.description}
                    </p>
                  )}
                  {e.technologies.length > 0 && (
                    <div className="adm-item__sub">{e.technologies.join("、")}</div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {parsed.projects.length > 0 && (
        <section>
          <h3 className="oil-title-sm">项目</h3>
          <ul className="adm-list">
            {parsed.projects.map((p, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: 项目没有 id，名字可能是空的
                key={index}
                className="adm-item"
                style={{ alignItems: "flex-start" }}
              >
                <div className="adm-item__main">
                  <strong>{[p.name, p.role].filter(Boolean).join(" · ") || "没写名字"}</strong>
                  <div className="adm-item__sub">{period(p.start, p.end)}</div>
                  {p.description && (
                    <p className="adm-prewrap" style={{ margin: "var(--oil-space-2) 0 0" }}>
                      {p.description}
                    </p>
                  )}
                  {p.technologies.length > 0 && (
                    <div className="adm-item__sub">{p.technologies.join("、")}</div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {parsed.education.length > 0 && (
        <section>
          <h3 className="oil-title-sm">教育</h3>
          <ul className="adm-list">
            {parsed.education.map((e, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: 教育经历没有 id
              <li key={index} className="adm-item">
                <div className="adm-item__main">
                  <strong>{e.school ?? "没写学校"}</strong>
                  <div className="adm-item__sub">
                    {[e.degree, e.major, period(e.start, e.end)].filter(Boolean).join(" · ")}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {parsed.skills.length > 0 && (
        <section>
          <h3 className="oil-title-sm">简历里写的技能</h3>
          <div className="adm-chips">
            {parsed.skills.map((skill) => (
              <Chip key={skill}>{skill}</Chip>
            ))}
          </div>
        </section>
      )}
      <Bullets title="证书" items={parsed.certifications} />
      <Bullets title="获奖" items={parsed.awards} />
      {(intention.length > 0 || parsed.languages.length > 0) && (
        <KeyValue
          items={[
            ...(intention.length > 0
              ? [["求职意向", intention.join(" · ")] as [string, string]]
              : []),
            ...(parsed.languages.length > 0
              ? [["语言", parsed.languages.join("、")] as [string, string]]
              : []),
          ]}
        />
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
  const pick = (c: Candidate): CandidateForm => ({
    name: c.name,
    phone: c.phone ?? null,
    email: c.email ?? null,
    current_title: c.current_title ?? null,
    current_company: c.current_company ?? null,
    location: c.location ?? null,
    notes: c.notes ?? null,
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

/** 一份简历：文件名、状态，以及重读、下载、删除 */
function ResumeRow({ resume, candidateId }: { resume: Resume; candidateId: string }) {
  const toast = useToast();
  const reparse = useReparseResume(candidateId);
  const remove = useDeleteResume(candidateId);
  const [deleting, setDeleting] = useState(false);
  const reading = resume.parse_status === "pending" || resume.parse_status === "running";
  return (
    <li className="adm-item" style={{ alignItems: "flex-start" }}>
      <FileText aria-hidden="true" size={18} />
      <div className="adm-item__main">
        <div className="adm-ellipsis">
          {resume.file_name ?? "文字简历"}
          {resume.is_primary && (
            <>
              {" "}
              <Badge plain>主简历</Badge>
            </>
          )}
        </div>
        <div className="adm-item__sub">
          {[
            resume.size_bytes == null ? null : fileSize(resume.size_bytes),
            relativeTime(resume.created_at),
          ]
            .filter(Boolean)
            .join(" · ")}
        </div>
        {resume.parse_error && <div className="adm-item__sub">{resume.parse_error}</div>}
        <div className="oil-row oil-row--wrap adm-row-actions">
          {resume.has_file && !isMock && (
            <a
              className="adm-link"
              href={`/api/v1/resumes/${resume.id}/file`}
              target="_blank"
              rel="noreferrer"
            >
              看原件
            </a>
          )}
          <button
            type="button"
            className="adm-link"
            disabled={reading || reparse.isPending}
            onClick={() =>
              reparse.mutate(resume.id, {
                onSuccess: () => toast("正在重新读这份简历"),
                onError: (e) => toast(errorMessage(e)),
              })
            }
          >
            重新读
          </button>
          <button type="button" className="adm-link" onClick={() => setDeleting(true)}>
            删除
          </button>
        </div>
      </div>
      <Badge tone={parseStatusTone[resume.parse_status]} plain>
        {parseStatusLabel[resume.parse_status]}
      </Badge>
      <Confirm
        open={deleting}
        title={`删除「${resume.file_name ?? "文字简历"}」？`}
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setDeleting(false)}
        onConfirm={() =>
          remove.mutate(resume.id, {
            onSuccess: () => {
              toast("已删除");
              setDeleting(false);
            },
            onError: (e) => {
              toast(errorMessage(e));
              setDeleting(false);
            },
          })
        }
      >
        {resume.is_primary
          ? "这是主简历，出题和画像都用它。删掉之后会改用剩下的简历里最新的一份。"
          : "删掉之后找不回来。"}
      </Confirm>
    </li>
  );
}

function Detail({ candidate }: { candidate: CandidateDetail }) {
  const navigate = useNavigate();
  const toast = useToast();
  const resumes = useResumes(candidate.id);
  const interviews = useInterviews({ candidate_id: candidate.id, page_size: 50 });
  const upload = useUploadResume(candidate.id);
  const reparse = useReparseResume(candidate.id);
  const regenerate = useRegenerateProfile(candidate.id);
  const remove = useDeleteCandidate();
  const [editing, setEditing] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const primary = candidate.primary_resume;
  const reading = primary?.parse_status === "pending" || primary?.parse_status === "running";
  const failed = primary?.parse_status === "failed";
  const generating = profilePending(candidate) || regenerate.isPending;
  const hasProfile = Boolean(candidate.profile_updated_at || candidate.profile.summary);
  const parsedResume = useResume(primary?.parse_status === "done" ? primary.id : null);
  const years = candidate.profile.years_of_experience;

  const regenerateProfile = () =>
    regenerate.mutate(undefined, {
      onSuccess: () => toast("画像已更新"),
      onError: (e) => toast(errorMessage(e)),
    });

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
      {candidate.possible_duplicates.length > 0 && (
        <Banner tone="warning" title="可能和这些候选人是同一个人" className="adm-gap-below">
          <DuplicateList items={candidate.possible_duplicates} />
        </Banner>
      )}
      <div className="adm-split">
        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <Card>
            <CardHeader
              title="画像"
              actions={
                hasProfile &&
                primary?.parse_status === "done" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<RefreshCw aria-hidden="true" />}
                    loading={regenerate.isPending}
                    onClick={regenerateProfile}
                  >
                    重新生成
                  </Button>
                )
              }
            />
            <CardBody>
              {reading ? (
                <Banner tone="info" title="正在读简历">
                  大模型在读这份简历，一般几十秒。读完这里会自动出现画像。
                </Banner>
              ) : failed ? (
                <Banner
                  tone="danger"
                  title="简历没有读出来"
                  actions={
                    primary && (
                      <Button
                        size="sm"
                        loading={reparse.isPending}
                        onClick={() =>
                          reparse.mutate(primary.id, { onError: (e) => toast(errorMessage(e)) })
                        }
                      >
                        再读一次
                      </Button>
                    )
                  }
                >
                  {primary?.parse_error ??
                    "可能是文件损坏或者是扫描件看不清。可以重试，或者换一份文件上传。"}
                </Banner>
              ) : generating ? (
                <Banner tone="info" title="正在生成画像">
                  简历读完了，大模型在归纳这位候选人的水平和值得核实的地方，大概十秒。
                </Banner>
              ) : hasProfile ? (
                <Profile profile={candidate.profile} />
              ) : (
                <div className="oil-stack">
                  <p className="oil-muted" style={{ margin: 0 }}>
                    {primary
                      ? "还没有画像。可以让大模型按简历归纳一份，面试官出题时会用到。"
                      : "还没有画像。上传一份简历，大模型会读出经历、技能和值得核实的地方，面试官出题时也会用到。"}
                  </p>
                  <div>
                    {primary ? (
                      <Button variant="secondary" onClick={regenerateProfile}>
                        生成画像
                      </Button>
                    ) : (
                      <Button variant="secondary" onClick={() => setUploading(true)}>
                        上传简历
                      </Button>
                    )}
                  </div>
                </div>
              )}
            </CardBody>
          </Card>
          {primary?.parse_status === "done" && (
            <Card>
              <CardHeader title="简历内容" />
              <CardBody>
                <QueryView query={parsedResume}>
                  {(resume) =>
                    resume.parsed ? (
                      <ResumeContent parsed={resume.parsed} />
                    ) : (
                      <p className="oil-muted" style={{ margin: 0 }}>
                        这份简历里没有读出经历。
                      </p>
                    )
                  }
                </QueryView>
              </CardBody>
            </Card>
          )}
        </div>

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
                    {years == null ? "工作年限还不知道" : `工作 ${years} 年`}
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
                    candidate.notes ? (
                      <span key="notes" className="adm-prewrap">
                        {candidate.notes}
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
                        <ResumeRow key={r.id} resume={r} candidateId={candidate.id} />
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
          accept={RESUME_ACCEPT}
          title={upload.isPending ? "正在上传…" : "把简历拖到这里，或者点一下选择文件"}
          hint={`新简历读完后，画像会按它更新。${RESUME_HINT}`}
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
        删掉之后列表里不再显示这位候选人，已经安排的面试不受影响。
      </Confirm>
    </>
  );
}

export function CandidateDetailPage() {
  const { id } = useParams();
  const candidate = useCandidate(id);
  return <QueryView query={candidate}>{(c) => <Detail candidate={c} />}</QueryView>;
}
