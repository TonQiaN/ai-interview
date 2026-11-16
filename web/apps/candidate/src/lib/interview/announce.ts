import type { RunnerPhase } from "./runner";

/**
 * 面试中读屏软件现在该念哪一句。返回空字符串就是什么也不念。
 *
 * 画面上的提示（「请听题，听完直接回答」这类）一换就念的话，面试官开口的头两三秒里
 * 读屏软件也在说话，题目的开头就听不清了。所以面试官在说话、在准备下一句的时候不念；
 * 轮到候选人了、面试停住了、要等一等的时候才念。
 *
 * 面试官的语音放不出来、只能看文字的时候，把这句话的文字也念出来：看不见画面的候选人
 * 没有别的办法知道面试官说了什么。
 */
export function announcement(phase: RunnerPhase, hint: string, caption: string | null): string {
  if (phase === "preparing" || phase === "asking") return "";
  if (phase === "reading") return [hint, caption ?? ""].filter((part) => part !== "").join(" ");
  return hint;
}
