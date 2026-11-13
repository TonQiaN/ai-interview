import {
  Banner,
  Button,
  buttonClass,
  Card,
  CardBody,
  Field,
  Input,
  PageHeader,
  Textarea,
  useToast,
} from "@ai-interview/ui";
import { type FormEvent, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { errorMessage } from "../api/http";
import { useConfigDefaults, useSaveTemplate, useTemplate } from "../api/queries";
import type { InterviewConfig } from "../api/types";
import { ConfigForm } from "../components/ConfigForm";
import { LoadError, Loading } from "../components/common";

interface Draft {
  name: string;
  description: string;
  config: InterviewConfig;
}

/** 新建或编辑一个面试设置模板 */
export function TemplateEditPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const toast = useToast();
  const existing = useTemplate(id);
  const defaults = useConfigDefaults();
  const save = useSaveTemplate(id);
  const [draft, setDraft] = useState<Draft | null>(null);

  // 数据到了之后填一次表单，之后以用户改的为准
  useEffect(() => {
    if (draft) return;
    if (id && existing.data)
      setDraft({
        name: existing.data.name,
        description: existing.data.description ?? "",
        config: existing.data.config,
      });
    if (!id && defaults.data) setDraft({ name: "", description: "", config: defaults.data.config });
  }, [id, existing.data, defaults.data, draft]);

  const failed = id ? existing.error : defaults.error;
  if (failed)
    return (
      <LoadError error={failed} onRetry={() => (id ? existing.refetch() : defaults.refetch())} />
    );
  if (!draft) return <Loading />;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!draft) return;
    save.mutate(
      {
        name: draft.name.trim(),
        description: draft.description.trim() || null,
        config: draft.config,
      },
      {
        onSuccess: () => {
          toast(id ? "模板已保存" : "模板已建好");
          navigate("/templates");
        },
      },
    );
  }

  return (
    <form onSubmit={submit}>
      <PageHeader
        title={id ? "编辑模板" : "新建模板"}
        crumbs={<Link to="/templates">面试设置</Link>}
        actions={
          <>
            <Link to="/templates" className={buttonClass({ variant: "ghost" })}>
              取消
            </Link>
            <Button type="submit" loading={save.isPending} disabled={!draft.name.trim()}>
              保存
            </Button>
          </>
        }
      />
      {save.error && (
        <Banner tone="danger" title="没有保存成功">
          {errorMessage(save.error)}
        </Banner>
      )}
      <Card className="adm-section">
        <CardBody>
          <div className="adm-form-grid">
            <Field label="模板名称" required className="adm-span">
              {({ id: fieldId }) => (
                <Input
                  id={fieldId}
                  value={draft.name}
                  placeholder="比如：技术岗初筛（含逻辑题）"
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              )}
            </Field>
            <Field label="什么时候用它" className="adm-span">
              {({ id: fieldId }) => (
                <Textarea
                  id={fieldId}
                  rows={2}
                  value={draft.description}
                  onChange={(e) => setDraft({ ...draft, description: e.target.value })}
                />
              )}
            </Field>
          </div>
        </CardBody>
      </Card>
      <div className="adm-section">
        <ConfigForm value={draft.config} onChange={(config) => setDraft({ ...draft, config })} />
      </div>
    </form>
  );
}
