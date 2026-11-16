import { Banner, Button, Dialog, Field, Input, Textarea, useToast } from "@ai-interview/ui";
import { useEffect, useState } from "react";
import { errorMessage } from "../api/http";
import { useSavePlan } from "../api/queries";
import type { InterviewPlan } from "../api/types";
import { addFollowUp, DEFAULT_ASK_WHEN, type FollowUpDraft } from "../lib/plan";

const BLANK: FollowUpDraft = { text: "", askWhen: "" };

/**
 * 给一道题加一句自己写的追问。不调模型：措辞就是 HR 写的，
 * 保存以后后端只给这一句合成语音，别的题和已经合成好的都不动。
 */
export function AddFollowUpDialog({
  interviewId,
  plan,
  target,
  conversational,
  onClose,
}: {
  interviewId: string;
  plan: InterviewPlan;
  /** 给哪道题加；null 时窗口关着 */
  target: { key: string; name: string; text: string } | null;
  /** 这场面试是不是实时对话：追问在两种方式下用法不一样，说明跟着变 */
  conversational: boolean;
  onClose: () => void;
}) {
  const save = useSavePlan(interviewId);
  const toast = useToast();
  const [draft, setDraft] = useState(BLANK);
  const open = target !== null;

  // 每次打开都从空的开始，上一次没保存的内容和报错不带过来
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开的那一下清
  useEffect(() => {
    if (!open) return;
    setDraft(BLANK);
    save.reset();
  }, [open]);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      unsaved={[draft.text, draft.askWhen].some((value) => value.trim() !== "")}
      title={target ? `给${target.name}加一句追问` : "加一句追问"}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            disabled={draft.text.trim() === "" || !target}
            loading={save.isPending}
            onClick={() => {
              if (!target) return;
              save.mutate(addFollowUp(plan, target.key, draft), {
                onSuccess: () => {
                  toast("加好了，正在合成这一句的语音");
                  onClose();
                },
              });
            }}
          >
            加进去并合成语音
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        {target && (
          <p className="oil-muted" style={{ margin: 0 }}>
            这道题是：{target.text}
          </p>
        )}
        <p className="oil-muted" style={{ margin: 0 }}>
          {conversational
            ? "这场面试是实时对话：面试官听完回答现场组织追问，你写的这一句是给它的方向，不一定照原话问。"
            : "候选人答完这道题，回答符合下面写的情况时，面试官照你写的原话追问这一句。"}
          重新出题时它不会保留。
        </p>
        <Field label="面试官会这样追问">
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={3}
              value={draft.text}
              onChange={(e) => setDraft((old) => ({ ...old, text: e.target.value }))}
            />
          )}
        </Field>
        <Field
          label="什么情况下问（可以不填）"
          hint={`写一句就行，比如「只说了团队做了什么，没说自己做了什么」。不会念给候选人。不填就是「${DEFAULT_ASK_WHEN}」。`}
        >
          {({ id, describedBy }) => (
            <Input
              id={id}
              aria-describedby={describedBy}
              value={draft.askWhen}
              onChange={(e) => setDraft((old) => ({ ...old, askWhen: e.target.value }))}
            />
          )}
        </Field>
      </div>
    </Dialog>
  );
}
