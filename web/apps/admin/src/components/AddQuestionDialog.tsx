import { Banner, Button, Dialog, Field, Input, Select, Textarea, useToast } from "@ai-interview/ui";
import { useEffect, useState } from "react";
import { errorMessage } from "../api/http";
import { useSavePlan } from "../api/queries";
import type { InterviewPlan, QuestionCategory } from "../api/types";
import { categoryLabel, duration } from "../lib/format";
import { addQuestion, answerTimeChoices, type QuestionDraft } from "../lib/plan";

/** 这场面试没有列考察维度时，新题记在这个名字下面 */
const FALLBACK_COMPETENCY = "综合能力";

/**
 * 题目出好以后再加一道自己的题。不调模型：措辞就是 HR 写的，
 * 保存以后后端只给这一句合成语音，别的题和已经合成好的都不动。
 */
export function AddQuestionDialog({
  interviewId,
  plan,
  maxAnswerSec,
  open,
  onClose,
}: {
  interviewId: string;
  plan: InterviewPlan;
  /** 设置里的单题最长时间，预计回答时间不能比它长 */
  maxAnswerSec: number;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSavePlan(interviewId);
  const toast = useToast();
  const competencies = (plan.brief.competencies ?? []).map((c) => c.name);
  const choices = answerTimeChoices(maxAnswerSec);
  const blank = (): QuestionDraft => ({
    text: "",
    intent: "",
    competency: competencies[0] ?? FALLBACK_COMPETENCY,
    category: "professional",
    expectedAnswerSec: choices.includes(120) ? 120 : (choices.at(-1) ?? maxAnswerSec),
  });
  const [draft, setDraft] = useState(blank);
  const set = (patch: Partial<QuestionDraft>) => setDraft((old) => ({ ...old, ...patch }));

  // 每次打开都从空的开始，上一次没保存的内容和报错不带过来
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开的那一下清
  useEffect(() => {
    if (!open) return;
    setDraft(blank());
    save.reset();
  }, [open]);

  const ready = draft.text.trim() !== "" && draft.intent.trim() !== "";

  return (
    <Dialog
      open={open}
      onClose={onClose}
      unsaved={draft.text.trim() !== "" || draft.intent.trim() !== ""}
      title="加一道题"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            disabled={!ready}
            loading={save.isPending}
            onClick={() =>
              save.mutate(addQuestion(plan, draft), {
                onSuccess: () => {
                  toast("加好了，正在合成这一句的语音");
                  onClose();
                },
              })
            }
          >
            加进去并合成语音
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <p className="oil-muted" style={{ margin: 0 }}>
          这道题排在主问题的最后，面试官照你写的原话问。时间紧的时候别的题会被跳过，这道不会；只有整场面试已经超出预期时长四分之一，才会不问就收尾。重新出题时它不会保留；想每场都问，把它写进设置里「必须问的题」。
        </p>
        <Field label="面试官会这样问">
          {({ id, describedBy }) => (
            <Textarea
              id={id}
              aria-describedby={describedBy}
              rows={4}
              value={draft.text}
              onChange={(e) => set({ text: e.target.value })}
            />
          )}
        </Field>
        <Field
          label="想考察什么"
          hint="写一句就行。不会念给候选人，面试官判断回答和写报告时会用到。"
        >
          {({ id, describedBy }) => (
            <Input
              id={id}
              aria-describedby={describedBy}
              value={draft.intent}
              onChange={(e) => set({ intent: e.target.value })}
            />
          )}
        </Field>
        {competencies.length > 0 && (
          <Field label="考察哪个维度" hint="报告里这道题的回答算在这个维度下面。">
            {({ id, describedBy }) => (
              <Select
                id={id}
                aria-describedby={describedBy}
                value={draft.competency}
                onChange={(e) => set({ competency: e.target.value })}
              >
                {competencies.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </Select>
            )}
          </Field>
        )}
        <div className="oil-row oil-row--wrap" style={{ alignItems: "flex-start" }}>
          <Field label="题目类型">
            {({ id, describedBy }) => (
              <Select
                id={id}
                aria-describedby={describedBy}
                value={draft.category}
                onChange={(e) => set({ category: e.target.value as QuestionCategory })}
              >
                {(Object.keys(categoryLabel) as QuestionCategory[]).map((c) => (
                  <option key={c} value={c}>
                    {categoryLabel[c]}
                  </option>
                ))}
              </Select>
            )}
          </Field>
          <Field label="预计回答多久">
            {({ id, describedBy }) => (
              <Select
                id={id}
                aria-describedby={describedBy}
                value={String(draft.expectedAnswerSec)}
                onChange={(e) => set({ expectedAnswerSec: Number(e.target.value) })}
              >
                {choices.map((sec) => (
                  <option key={sec} value={sec}>
                    {duration(sec * 1000)}
                  </option>
                ))}
              </Select>
            )}
          </Field>
        </div>
      </div>
    </Dialog>
  );
}
