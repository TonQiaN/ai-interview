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

/**
 * 实时对话：候选人说完一段，面试官现场接话（追问是现场生成、现场合成的）。
 *
 * 同样不规定大模型怎么反应，只看链路，并量出候选人说完到面试官开口等了多久。
 * 云端没有真的语音合成（三个引擎只能在 Mac 上跑），这里的合成是瞬间完成的测试音，
 * 所以量出来的数字不含合成的时间。
 */
test("真模型：实时对话，面试官现场接话", async ({ page }) => {
  // 候选人停顿时页面会先问服务端「说完了吗」，把每次的结论记下来
  const verdicts: string[] = [];
  page.on("response", (response) => {
    if (!response.url().endsWith("/api/v1/room/pause-check")) return;
    response.json().then(
      (body: { verdict?: string }) => verdicts.push(body.verdict ?? "failed"),
      () => verdicts.push("failed"),
    );
  });
  // 面试官说话时候选人插话了几次（假麦克风是循环播放的，说话那一段碰上面试官在说就是插话）
  const cuts: number[] = [];
  page.on("request", (request) => {
    if (!request.url().endsWith("/api/v1/room/advance")) return;
    const body = request.postDataJSON() as { interrupted_played_ms?: number | null } | null;
    if (typeof body?.interrupted_played_ms === "number") cuts.push(body.interrupted_played_ms);
  });
  const interview = await enterInterview(page, { silenceSec: 3, minSec: 2, realtime: true });
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 8 * 60_000 });

  const state = await expectRecordingsReady(interview, 1);
  console.log(timeline(state));
  expect(state.status).toBe("completed");
  expect(spokenKeys(state)[0]).toBe("opening");

  const answers = state.turns.filter(
    (turn) => turn.speaker === "candidate" && turn.end_reason === "silence",
  );
  expect(answers.length).toBeGreaterThan(0);
  for (const answer of answers) {
    expect(answer.has_text, `回答 ${answer.utterance_key} 没有文字`).toBe(true);
    expect(answer.stt_engine).toBe("sherpa");
  }

  // 候选人说完最后一个字，到面试官下一句开始播，隔了多久。里面含判断「说完了」要等的那段静音
  const waits = state.turns.flatMap((turn, index) => {
    const next = state.turns[index + 1];
    return turn.speaker === "candidate" && turn.end_ms !== null && next
      ? [next.start_ms - turn.end_ms]
      : [];
  });
  console.log(
    `候选人说完到面试官开口：${waits.length} 轮，中位数 ${percentile(waits, 0.5)} 毫秒，` +
      `p95 ${percentile(waits, 0.95)} 毫秒，最长 ${Math.max(...waits)} 毫秒`,
  );
  const count = (verdict: string) => verdicts.filter((value) => value === verdict).length;
  console.log(
    `停顿时先问了 ${verdicts.length} 次：说完了 ${count("done")}，没说完 ${count("wait")}，` +
      `不知道 ${count("unknown")}，没问到 ${count("failed")}`,
  );
  console.log(
    `候选人插话 ${cuts.length} 次，面试官的话断在第 ${cuts.join("、") || "无"} 毫秒（重试的上报也算在内）`,
  );
  // 面试官现场说的话：key 是「题号.p几」（追问）或「题号.r几」（回应）
  const improvised = spokenKeys(state).filter((key) => key !== null && /\.[pr]\d+$/.test(key));
  console.log(
    `现场生成的话：${improvised.length} 拍（${[...new Set(improvised)].join("、") || "无"}）`,
  );
});

function percentile(values: number[], fraction: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)] ?? 0;
}

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
