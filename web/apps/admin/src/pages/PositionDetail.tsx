import {
  Badge,
  Banner,
  Button,
  Card,
  CardBody,
  CardHeader,
  Dialog,
  KeyValue,
  PageHeader,
  useToast,
} from "@ai-interview/ui";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { errorMessage } from "../api/http";
import { useDeletePosition, useInterviews, usePosition, useSavePosition } from "../api/queries";
import type { Position, PositionExtra } from "../api/types";
import { Confirm, DecisionBadge, Person, QueryView, StatusBadge } from "../components/common";
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
  const interviews = useInterviews({ position_id: position.id, page_size: 50 });
  const remove = useDeletePosition();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);

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
            <span>
              {[position.extra.client_name, position.department, position.location]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </span>
        }
        actions={
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
        }
      />
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
            <CardHeader title="这个岗位的面试" />
            <CardBody>
              <QueryView query={interviews}>
                {({ items }) =>
                  items.length === 0 ? (
                    <p className="oil-muted" style={{ margin: 0 }}>
                      还没有人面过这个岗位。
                    </p>
                  ) : (
                    <ul className="adm-list">
                      {items.map((i) => (
                        <li key={i.id} className="adm-item">
                          <div className="adm-item__main">
                            <Person
                              name={i.candidate_name}
                              sub={relativeTime(i.updated_at)}
                              to={`/interviews/${i.id}`}
                            />
                          </div>
                          <DecisionBadge value={i.decision} />
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
              删除岗位
            </Button>
          </div>
        </div>
      </div>

      <EditDialog position={position} open={editing} onClose={() => setEditing(false)} />
      <Confirm
        open={deleting}
        title={`删除「${position.title}」？`}
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setDeleting(false)}
        onConfirm={() =>
          remove.mutate(position.id, {
            onSuccess: () => {
              toast("已删除");
              navigate("/positions", { replace: true });
            },
            onError: (e) => {
              toast(errorMessage(e));
              setDeleting(false);
            },
          })
        }
      >
        删掉之后列表里不再显示这个岗位，已经安排的面试不受影响。
      </Confirm>
    </>
  );
}

export function PositionDetailPage() {
  const { id } = useParams();
  const position = usePosition(id);
  return <QueryView query={position}>{(p) => <Detail position={p} />}</QueryView>;
}
