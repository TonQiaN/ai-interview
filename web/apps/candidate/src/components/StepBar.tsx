import { Steps } from "@ai-interview/ui";
import { useText } from "../text";

/**
 * 进入面试之前那几页顶上的步骤条。
 *
 * 窄的版式里步骤的名字不显示，读屏软件读到的只有「1、2、3、4」。这里把整条当成一张图，
 * 配一句话：「第 2 步，共 4 步：同意」。
 */
export function StepBar({ current }: { current: number }) {
  const t = useText();
  return (
    <div role="img" aria-label={t.stepOf(current + 1, t.steps.length, t.steps[current] ?? "")}>
      <Steps compact steps={t.steps} current={current} />
    </div>
  );
}
