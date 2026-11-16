import {
  Badge,
  Banner,
  Button,
  buttonClass,
  Card,
  CardBody,
  CardHeader,
  Dialog,
  KeyValue,
  useToast,
} from "@ai-interview/ui";
import { Archive, ArchiveRestore, Download, Pencil, Plus, Scale, Send } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { errorMessage } from "../api/http";
import {
  useDeletePosition,
  useInterviews,
  usePosition,
  useRestorePosition,
  useSavePosition,
} from "../api/queries";
import type { Position, PositionExtra } from "../api/types";
import { BatchInvitesDialog } from "../components/BatchInvitesDialog";
import { Confirm, Pager, Person, QueryView } from "../components/common";
import { PageHeader } from "../components/PageHeader";
import { PrepareAll } from "../components/PrepareAll";
import { RaasApplicantsDialog } from "../components/RaasApplicantsDialog";
import { hasVerdict, InterviewTags } from "../components/ReportVerdict";
import { sameForm } from "../lib/form";
import { fullTime, positionStatusLabel, positionStatusTone, relativeTime } from "../lib/format";
import { PositionFields, type PositionForm } from "./Positions";

const pick = (p: Position): PositionForm => ({
  title: p.title,
  department: p.department ?? null,
  location: p.location ?? null,
  employment_type: p.employment_type ?? null,
  seniority: p.seniority ?? null,
  description: p.description,
  requirements: p.requirements,
  status: p.status,
  // 表单里没有的字段（关键词、加分项这些）原样带回去，不然保存的时候会丢
  extra: p.extra,
});

function EditDialog({
  position,
  open,
  onClose,
}: {
  position: Position;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSavePosition(position.id);
  const toast = useToast();
  const [value, setValue] = useState(() => pick(position));
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开时把表单重置成最新的数据
  useEffect(() => {
    if (open) setValue(pick(position));
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
      unsaved={!sameForm(value, pick(position))}
      wide
      title="改岗位"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            type="submit"
            form="edit-position"
            loading={save.isPending}
            disabled={!value.title.trim()}
          >
            保存
          </Button>
        </>
      }
    >
      <form id="edit-position" onSubmit={submit} className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <PositionFields value={value} onChange={setValue} />
      </form>
    </Dialog>
  );
}

const extraLists: Array<[keyof PositionExtra, string]> = [
  ["hard_requirements", "硬性要求"],
  ["interview_focus", "面试重点考察"],
  ["must_have_skills", "必须会的技能"],
  ["nice_to_have_skills", "加分的技能"],
  ["nice_to_have", "加分项"],
  ["negative_requirements", "明确不要的"],
];

const extraTexts: Array<[keyof PositionExtra, string]> = [
  ["experience_requirement", "工作年限"],
  ["degree_requirement", "学历"],
  ["language_requirement", "语言"],
  ["benefits", "福利"],
  ["evaluation_rules", "评估标准"],
];

/** 岗位的结构化信息：出题和报告会用到的要点。都没有就不显示这张卡 */
function ExtraCard({ extra }: { extra: PositionExtra }) {
  const lists = extraLists
    .map(([key, label]) => [label, extra[key] as string[]] as const)
    .filter(([, items]) => items.length > 0);
  const texts = extraTexts
    .map(([key, label]) => [label, extra[key] as string | null] as const)
    .filter(([, value]) => Boolean(value));
  if (lists.length === 0 && texts.length === 0) return null;
  return (
    <Card>
      <CardHeader title="要点" />
      <CardBody>
        <div className="oil-stack">
          <p className="oil-muted adm-note">出题和写报告的时候会参考这些。</p>
          {lists.map(([label, items]) => (
            <div key={label}>
              <div className="adm-label">{label}</div>
              <ul className="adm-plain-list">
                {items.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
          ))}
          {texts.length > 0 && (
            <KeyValue items={texts.map(([label, value]) => [label, value ?? "—"])} />
          )}
        </div>
      </CardBody>
    </Card>
  );
}

function Detail({ position }: { position: Position }) {
  const navigate = useNavigate();
  const toast = useToast();
  // 一个岗位下的人是要拿来比的：综合分高的在前，还没有报告的排在后面。
  // 从 RAAS 导入一批投递以后一个岗位会有几十上百场，一页 50 场，多了可以翻页
  const [page, setPage] = useState(1);
  const listed = { position_id: position.id, page_size: 50, sort: "-score" } as const;
  const interviews = useInterviews({ ...listed, page });
  // 有报告的都排在前面，够不够两份拿来对比只用看第一页（在第一页时和上面是同一个请求）
  const topInterviews = useInterviews({ ...listed, page: 1 });
  // 题目准备好、还没发链接的。一批安排下来常常有十几二十场，一场一场点进去发太慢，可以一起生成
  const ready = useInterviews({ position_id: position.id, status: "ready", page_size: 100 });
  const readyTotal = ready.data?.total ?? 0;
  const [inviting, setInviting] = useState(false);
  const remove = useDeletePosition();
  const restore = useRestorePosition();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [importing, setImporting] = useState(false);
  // 只有从 RAAS 导入的岗位才查得到 RAAS 里谁投过
  const fromRaas = position.source === "raas" && Boolean(position.external_ref);
  // 归档的岗位后端不让改，也不让再安排面试，这些按钮就不给了
  const archived = Boolean(position.archived_at);

  return (
    <>
      <PageHeader
        crumbs={<Link to="/positions">岗位</Link>}
        title={position.title}
        description={
          <span className="oil-row oil-row--wrap">
            <Badge tone={positionStatusTone[position.status]}>
              {positionStatusLabel[position.status]}
            </Badge>
            {archived && <Badge plain>已归档</Badge>}
            <span>
              {[position.extra.client_name, position.department, position.location]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </span>
        }
        actions={
          !archived && (
            <>
              <Button
                variant="secondary"
                icon={<Pencil aria-hidden="true" />}
                onClick={() => setEditing(true)}
              >
                改岗位
              </Button>
              <Button
                icon={<Plus aria-hidden="true" />}
                disabled={position.status !== "open"}
                onClick={() => navigate(`/interviews/new?position=${position.id}`)}
              >
                安排面试
              </Button>
            </>
          )
        }
      />
      {archived && (
        <Banner
          tone="info"
          title="这个岗位已经归档"
          className="adm-gap-below"
          actions={
            <Button
              variant="secondary"
              size="sm"
              icon={<ArchiveRestore />}
              loading={restore.isPending}
              onClick={() =>
                restore.mutate(position.id, {
                  onSuccess: () => toast("已恢复"),
                  onError: (e) => toast(errorMessage(e)),
                })
              }
            >
              恢复
            </Button>
          }
        >
          归档的岗位不在列表里显示，不能再修改，也不能再安排面试。岗位的内容和已经安排的面试都还在，在岗位列表勾上「带上已归档的」能找到。归档错了，点「恢复」就回到列表里。
        </Banner>
      )}
      <div className="adm-split">
        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <Card>
            <CardHeader title="岗位职责" />
            <CardBody>
              <div className="adm-prewrap">
                {position.description || (
                  <span className="oil-muted">还没写。面试官出题需要它。</span>
                )}
              </div>
            </CardBody>
          </Card>
          <Card>
            <CardHeader title="任职要求" />
            <CardBody>
              <div className="adm-prewrap">
                {position.requirements || <span className="oil-muted">还没写。</span>}
              </div>
            </CardBody>
          </Card>
          <ExtraCard extra={position.extra} />
        </div>

        <div
          className="oil-stack"
          style={{ "--oil-gap": "var(--oil-space-5)" } as React.CSSProperties}
        >
          <Card>
            <CardBody>
              <KeyValue
                items={[
                  ["用工形式", position.employment_type ?? "—"],
                  ["职级", position.seniority ?? "—"],
                  ["薪资范围", position.extra.salary_range ?? "—"],
                  ["招几个人", position.extra.headcount ?? "—"],
                  [
                    "来源",
                    position.source === "raas"
                      ? `RAAS（${position.external_ref ?? "没有编号"}）`
                      : "自己写的",
                  ],
                  ["建于", fullTime(position.created_at)],
                ]}
              />
            </CardBody>
          </Card>
          <Card>
            <CardHeader
              title="这个岗位的面试"
              actions={
                <>
                  {/* 有两场以上出了报告才比得起来 */}
                  {(topInterviews.data?.items.filter(hasVerdict).length ?? 0) >= 2 && (
                    <Link
                      to={`/positions/${position.id}/comparison`}
                      className={buttonClass({ variant: "secondary", size: "sm" })}
                    >
                      <Scale aria-hidden="true" />
                      对比候选人
                    </Link>
                  )}
                  {fromRaas && !archived && (
                    <Button
                      variant="secondary"
                      size="sm"
                      icon={<Download aria-hidden="true" />}
                      onClick={() => setImporting(true)}
                    >
                      导入 RAAS 的投递
                    </Button>
                  )}
                </>
              }
            />
            <CardBody>
              {!archived && <PrepareAll positionId={position.id} />}
              {readyTotal > 0 && !archived && (
                <Banner
                  tone="info"
                  className="adm-gap-below"
                  actions={
                    <Button
                      size="sm"
                      icon={<Send aria-hidden="true" />}
                      onClick={() => setInviting(true)}
                    >
                      {readyTotal > 1 ? "一起生成链接" : "生成链接"}
                    </Button>
                  }
                >
                  {readyTotal} 场的题目准备好了，还没发链接。
                </Banner>
              )}
              <QueryView query={interviews}>
                {({ items, total, page_size }) =>
                  total === 0 ? (
                    <p className="oil-muted" style={{ margin: 0 }}>
                      还没有人面过这个岗位。
                    </p>
                  ) : (
                    <>
                      {(items.some(hasVerdict) || total > page_size) && (
                        <p className="oil-muted adm-note">综合分高的在前，还没有报告的排在后面。</p>
                      )}
                      <ul className="adm-list">
                        {items.map((i) => (
                          <li key={i.id} className="adm-item adm-item--wrap">
                            <div className="adm-item__main">
                              <Person
                                name={i.candidate_name}
                                sub={relativeTime(i.updated_at)}
                                to={`/interviews/${i.id}`}
                              />
                            </div>
                            <InterviewTags interview={i} />
                          </li>
                        ))}
                      </ul>
                      {total > page_size && (
                        <Pager page={page} pageSize={page_size} total={total} onPage={setPage} />
                      )}
                    </>
                  )
                }
              </QueryView>
            </CardBody>
          </Card>
          {!archived && (
            <div>
              <Button
                variant="ghost"
                size="sm"
                icon={<Archive aria-hidden="true" />}
                onClick={() => setDeleting(true)}
              >
                归档
              </Button>
            </div>
          )}
        </div>
      </div>

      <EditDialog position={position} open={editing} onClose={() => setEditing(false)} />
      <BatchInvitesDialog
        interviews={ready.data?.items ?? []}
        total={readyTotal}
        open={inviting}
        onClose={() => setInviting(false)}
      />
      {fromRaas && (
        <RaasApplicantsDialog
          position={position}
          open={importing}
          onClose={() => setImporting(false)}
        />
      )}
      <Confirm
        open={deleting}
        title={`归档「${position.title}」？`}
        confirmLabel="归档"
        loading={remove.isPending}
        onClose={() => setDeleting(false)}
        onConfirm={() =>
          remove.mutate(position.id, {
            onSuccess: () => {
              toast("已归档");
              navigate("/positions", { replace: true });
            },
            onError: (e) => {
              toast(errorMessage(e));
              setDeleting(false);
            },
          })
        }
      >
        归档以后列表里不再显示这个岗位，安排面试时也选不到，已经安排的面试不受影响。岗位的内容还留着，没有删掉。
      </Confirm>
    </>
  );
}

export function PositionDetailPage() {
  const { id } = useParams();
  const position = usePosition(id);
  return <QueryView query={position}>{(p) => <Detail position={p} />}</QueryView>;
}
