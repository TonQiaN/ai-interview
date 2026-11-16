import { Banner, Button, Checkbox, Dialog, useToast } from "@ai-interview/ui";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { errorMessage } from "../api/http";
import { useSavePlan } from "../api/queries";
import type { InterviewPlan } from "../api/types";
import { canMoveQuestion, canSetReserve, moveQuestion, setRequired, setReserve } from "../lib/plan";

/**
 * 调主问题的先后、定哪些题时间紧也要问、主问题和备用题互换。
 *
 * 面试中时间不够时，没标必问的题会被跳过，越靠后越容易轮不上；备用题只在时间有富余时才问。
 * 一道题问不问得到就看这三样，所以放在一起调。
 * 在窗口里调好再一起存：每存一次面试都要回到「准备中」走一遍，一下一下存的话每点一次都要等。
 * 面试官要说的话一个字没变，存了以后不会重新合成语音。
 */
export function ArrangeQuestionsDialog({
  interviewId,
  plan,
  open,
  onClose,
}: {
  interviewId: string;
  plan: InterviewPlan;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSavePlan(interviewId);
  const toast = useToast();
  const [draft, setDraft] = useState(plan);
  /** 读屏软件念的那一句：挪了以后现在排第几 */
  const [said, setSaid] = useState("");
  const lists = useRef<HTMLDivElement>(null);
  /** 挪完以后焦点放回哪个按钮：先试刚才按的那个，它到头了不能再按就试反方向的 */
  const focusNext = useRef<string[] | null>(null);

  // 每次打开都从现在的题目开始，上一次没保存的改动和报错不带过来
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开的那一下重来
  useEffect(() => {
    if (!open) return;
    setDraft(plan);
    setSaid("");
    save.reset();
  }, [open]);

  // 挪了以后那一行换了位置，焦点会丢；放回去，只用键盘的人可以接着挪
  // biome-ignore lint/correctness/useExhaustiveDependencies: 题目的先后变了以后才放
  useEffect(() => {
    const wanted = focusNext.current;
    if (!wanted) return;
    focusNext.current = null;
    for (const name of wanted) {
      const button = lists.current?.querySelector<HTMLButtonElement>(
        `[data-act="${name}"]:not(:disabled)`,
      );
      if (button) {
        button.focus();
        break;
      }
    }
  }, [draft]);

  // 窗口关着的时候不摆题目：这时手里可能还是上一次打开时的那一份
  const shown = open ? draft.questions : [];
  const main = shown.filter((q) => !q.reserve);
  const reserve = shown.filter((q) => q.reserve);
  const changed = shown.some((q, index) => {
    const before = plan.questions[index];
    return before?.key !== q.key || before.required !== q.required || before.reserve !== q.reserve;
  });

  const move = (key: string, no: number, direction: -1 | 1) => {
    focusNext.current = [`${key}:${direction}`, `${key}:${-direction}`];
    setDraft((old) => moveQuestion(old, key, direction));
    setSaid(`挪到了第 ${no + direction} 题`);
  };

  // 换到另一边以后这一行到了另一个列表里，焦点跟过去，落在能把它换回来的那个按钮上
  const swap = (key: string, toReserve: boolean) => {
    focusNext.current = [`${key}:${toReserve ? "main" : "reserve"}`];
    setDraft((old) => setReserve(old, key, toReserve));
    setSaid(toReserve ? "改成了备用题" : `改成了主问题，排在第 ${main.length + 1} 题`);
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      unsaved={changed}
      title="调先后和必问"
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            取消
          </Button>
          <Button
            disabled={!changed}
            loading={save.isPending}
            onClick={() =>
              save.mutate(draft, {
                onSuccess: () => {
                  toast("改好了");
                  onClose();
                },
              })
            }
          >
            保存
          </Button>
        </>
      }
    >
      <div className="oil-stack">
        {save.error && <Banner tone="danger">{errorMessage(save.error)}</Banner>}
        <p className="oil-muted" style={{ margin: 0 }}>
          时间不够时，没勾「时间紧也要问」的题会被跳过，越靠后越容易轮不上。勾了的题一定会问，只有整场面试已经超出预期时长四分之一，才会不问就收尾。这里的改动不用重新合成语音。
          备用题只在时间有富余时才问；想让一道备用题轮得上，把它改成主问题。
        </p>
        <div className="oil-stack" ref={lists}>
          <ol className="adm-list" aria-label="主问题">
            {main.map((q, index) => {
              const no = index + 1;
              return (
                <li key={q.key} className="adm-item adm-arrange">
                  <span className="adm-step__dot" aria-hidden="true">
                    {no}
                  </span>
                  <div className="adm-item__main">
                    <p className="adm-arrange__text">{q.text}</p>
                    <div className="adm-arrange__options">
                      <Checkbox
                        label="时间紧也要问"
                        aria-label={`第 ${no} 题时间紧也要问`}
                        checked={q.required}
                        onChange={(e) => {
                          const required = e.target.checked;
                          setDraft((old) => setRequired(old, q.key, required));
                        }}
                      />
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`第 ${no} 题改成备用题`}
                        data-act={`${q.key}:reserve`}
                        disabled={!canSetReserve(draft, q.key, true)}
                        onClick={() => swap(q.key, true)}
                      >
                        改成备用题
                      </Button>
                    </div>
                  </div>
                  <div className="adm-arrange__moves">
                    <Button
                      size="sm"
                      variant="ghost"
                      iconOnly
                      icon={<ArrowUp aria-hidden="true" />}
                      aria-label={`第 ${no} 题上移`}
                      title="上移"
                      data-act={`${q.key}:-1`}
                      disabled={!canMoveQuestion(draft, q.key, -1)}
                      onClick={() => move(q.key, no, -1)}
                    />
                    <Button
                      size="sm"
                      variant="ghost"
                      iconOnly
                      icon={<ArrowDown aria-hidden="true" />}
                      aria-label={`第 ${no} 题下移`}
                      title="下移"
                      data-act={`${q.key}:1`}
                      disabled={!canMoveQuestion(draft, q.key, 1)}
                      onClick={() => move(q.key, no, 1)}
                    />
                  </div>
                </li>
              );
            })}
          </ol>
          {reserve.length > 0 && (
            <>
              <h3 className="oil-title-sm" style={{ margin: 0 }}>
                备用题
              </h3>
              <ol className="adm-list" aria-label="备用题">
                {reserve.map((q, index) => (
                  <li key={q.key} className="adm-item adm-arrange">
                    <span className="adm-step__dot" aria-hidden="true">
                      备
                    </span>
                    <div className="adm-item__main">
                      <p className="adm-arrange__text">{q.text}</p>
                      <div className="adm-arrange__options">
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={`第 ${index + 1} 道备用题改成主问题`}
                          data-act={`${q.key}:main`}
                          onClick={() => swap(q.key, false)}
                        >
                          改成主问题
                        </Button>
                      </div>
                    </div>
                  </li>
                ))}
              </ol>
            </>
          )}
        </div>
        <p className="oil-sr-only" aria-live="polite">
          {said}
        </p>
      </div>
    </Dialog>
  );
}
