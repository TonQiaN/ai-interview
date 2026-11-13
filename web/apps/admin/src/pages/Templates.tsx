import { Button, buttonClass, Card, CardBody, PageHeader, useToast } from "@ai-interview/ui";
import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";
import { errorMessage } from "../api/http";
import { useDeleteTemplate, useTemplates } from "../api/queries";
import type { InterviewTemplate } from "../api/types";
import { configSummary } from "../components/ConfigForm";
import { Confirm, Empty, QueryView } from "../components/common";
import { relativeTime } from "../lib/format";

/** 卡片上列几条最能说明这个模板的要求 */
function requirements(t: InterviewTemplate): string[] {
  return [...t.config.focus_instructions, ...t.config.must_ask_questions].slice(0, 4);
}

/** 面试设置的模板：把常用的时长、考察要求、思考强度存下来，安排面试时一选就好。 */
export function TemplatesPage() {
  const templates = useTemplates();
  const remove = useDeleteTemplate();
  const toast = useToast();
  const [deleting, setDeleting] = useState<InterviewTemplate | null>(null);

  return (
    <>
      <PageHeader
        title="面试设置"
        description="把常用的一套设置存成模板：面多久、重点考察什么、哪些题必须问、面试官想得多深。安排面试时选一个，还可以再按这一场调整。"
        actions={
          <Link to="/templates/new" className={buttonClass()}>
            <Plus aria-hidden="true" />
            新建模板
          </Link>
        }
      />
      <QueryView query={templates}>
        {({ items }) =>
          items.length === 0 ? (
            <Empty
              title="还没有模板"
              description="建一个模板，以后安排面试就不用每次都从头设置。"
            />
          ) : (
            <div className="adm-templates">
              {items.map((t) => (
                <Card key={t.id}>
                  <CardBody>
                    <div className="adm-dim__head">
                      <Link to={`/templates/${t.id}`} className="adm-link adm-template__name">
                        {t.name}
                      </Link>
                    </div>
                    {t.description && <p className="adm-template__desc">{t.description}</p>}
                    <p className="adm-item__sub">{configSummary(t.config)}</p>
                    {requirements(t).length > 0 && (
                      <ul className="adm-bullets adm-bullets--muted adm-template__focus">
                        {requirements(t).map((p) => (
                          <li key={p}>{p}</li>
                        ))}
                      </ul>
                    )}
                    <div className="adm-template__foot">
                      <span className="adm-item__sub">改于 {relativeTime(t.updated_at)}</span>
                      <span className="oil-row">
                        <Link
                          to={`/templates/${t.id}`}
                          className={buttonClass({ variant: "secondary", size: "sm" })}
                        >
                          编辑
                        </Link>
                        <Button
                          size="sm"
                          variant="ghost"
                          iconOnly
                          aria-label={`删除${t.name}`}
                          icon={<Trash2 aria-hidden="true" />}
                          onClick={() => setDeleting(t)}
                        />
                      </span>
                    </div>
                  </CardBody>
                </Card>
              ))}
            </div>
          )
        }
      </QueryView>
      <Confirm
        open={deleting !== null}
        title="删除这个模板？"
        confirmLabel="删除"
        danger
        loading={remove.isPending}
        onClose={() => setDeleting(null)}
        onConfirm={() => {
          if (!deleting) return;
          remove.mutate(deleting.id, {
            onSuccess: () => {
              toast("模板已删除");
              setDeleting(null);
            },
            onError: (e) => toast(errorMessage(e)),
          });
        }}
      >
        「{deleting?.name}」会被删除。已经用它安排好的面试不受影响。
      </Confirm>
    </>
  );
}
