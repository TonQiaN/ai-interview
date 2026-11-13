import { expect, test } from "@playwright/test";
import { enterInterview, expectRecordingsReady, type RoomState, spokenKeys } from "./helpers";

/**
 * 用真的语音识别模型和真的大模型走一场面试。平时不跑，要 E2E_REAL=1（见 playwright.config.ts）。
 *
 * 假麦克风循环放一段有人说话的录音，说完安静几秒。这段话和题目没有关系，
 * 所以这里不规定大模型该怎么反应（追问、换题、提前结束都可能），只看链路：
 * 浏览器录的声音传到了服务端，被识别成了文字，面试能走到结束。
 * 每一步用了多久会打印出来，看候选人答完到下一句开始要等几秒。
 */
test("真模型：候选人说的话被听出来，面试走到结束", async ({ page }) => {
  // 示例录音的句子之间有两秒多的停顿，静音时长设 3 秒才不会把一句话切成两题
  const interview = await enterInterview(page, { silenceSec: 3, minSec: 2 });
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 8 * 60_000 });

  const state = await expectRecordingsReady(interview, 1);
  console.log(timeline(state));
  expect(state.status).toBe("completed");
  expect(spokenKeys(state)[0]).toBe("opening");

  // 提问阶段、因为安静下来而结束的回答，都应该听出了文字，而且是真模型听的
  const answers = state.turns.filter(
    (turn) =>
      turn.speaker === "candidate" &&
      turn.utterance_key?.startsWith("q") &&
      turn.end_reason === "silence",
  );
  expect(answers.length).toBeGreaterThan(0);
  for (const answer of answers) {
    expect(answer.has_text, `回答 ${answer.utterance_key} 没有文字`).toBe(true);
    expect(answer.stt_engine).toBe("sherpa");
  }
});

/** 把一场面试的经过排成几行字：谁在什么时候说了哪一句，候选人答完之后等了多久。 */
function timeline(state: RoomState): string {
  const seconds = (ms: number | null) => (ms === null ? "?" : (ms / 1000).toFixed(1));
  const lines = [`结束原因：${state.end_reason}`];
  state.turns.forEach((turn, index) => {
    const who = turn.speaker === "interviewer" ? "面试官" : "候选人";
    let line = `${seconds(turn.start_ms).padStart(6)} → ${seconds(turn.end_ms).padStart(6)}  ${who}  ${turn.utterance_key}`;
    if (turn.speaker === "candidate") {
      line += `  ${turn.end_reason}  ${turn.has_text ? `有文字（${turn.stt_engine}）` : "没有文字"}`;
      const next = state.turns[index + 1];
      if (turn.end_ms !== null && next) {
        line += `  答完 ${seconds(next.start_ms - turn.end_ms)} 秒后面试官开口`;
      }
    }
    lines.push(line);
  });
  return lines.join("\n");
}
