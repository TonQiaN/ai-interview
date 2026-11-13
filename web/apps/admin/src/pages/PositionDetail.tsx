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
import type { Position, PositionInput } from "../api/types";
import { Confirm, Person, QueryView, RecommendationBadge, StatusBadge } from "../components/common";
import { fullTime, relativeTime } from "../lib/format";
import { PositionFields } from "./Positions";

const pick = (p: Position): PositionInput => ({
  title: p.title,
  department: p.department,
  location: p.location,
  employment_type: p.employment_type,
  salary_range: p.salary_range,
  headcount: p.headcount,
  description: p.description,
  requirements: p.requirements,
  status: p.status,
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
            <Badge tone={position.status === "open" ? "success" : "neutral"}>
              {position.status === "open" ? "在招" : "已关闭"}
            </Badge>
            <span>{[position.department, position.location].filter(Boolean).join(" · ")}</span>
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
                  ["薪资范围", position.salary_range ?? "—"],
                  ["招几个人", position.headcount ?? "—"],
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
                              name={i.candidate.name}
                              sub={relativeTime(i.updated_at)}
                              to={`/interviews/${i.id}`}
                            />
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
        有面试记录的岗位不能删，可以改成「已关闭」。
      </Confirm>
    </>
  );
}

export function PositionDetailPage() {
  const { id } = useParams();
  const position = usePosition(id);
  return <QueryView query={position}>{(p) => <Detail position={p} />}</QueryView>;
}
