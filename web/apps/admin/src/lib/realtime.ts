import type { ComponentStatus } from "../api/types";

/** 实时对话靠这两样：大模型现场决定怎么接话，语音识别听候选人说了什么 */
const needed: Record<string, string> = { llm: "大模型", stt: "语音识别" };

/**
 * 现在是什么拦着实时对话。返回不能用的那几样的名字，空的就是能用。
 *
 * 开始面试时少一样，后端会让这场面试按逐题问答进行（见 docs/architecture/api.md 的「实时对话」）。
 * 语音合成接的是假后端也照样能进行，所以不看它。
 */
export function realtimeBlockers(
  components: Pick<ComponentStatus, "name" | "state">[] | undefined,
): string[] {
  return (components ?? [])
    .filter((c) => c.name in needed && (c.state === "down" || c.state === "not_configured"))
    .map((c) => needed[c.name] as string);
}
