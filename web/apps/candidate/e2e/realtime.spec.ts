import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  expectRecordingsReady,
  passPreflight,
  roomState,
  spokenKeys,
  timeline,
} from "./helpers";

/**
 * 实时对话：候选人说完一段，面试官现场接话。
 *
 * 平时不调大模型，接话的是面试间自带的替身（`AI_INTERVIEW_ROOM_FAKE_LIVE_MODEL`，见 e2e/backend.sh）：
 * 它不看候选人说了什么，每道题固定追问一句，候选人再答完就进入下一题。所以这里测不出面试官接得好不好
 * （那要真模型，见 real.spec.ts），测的是页面和服务端在实时对话下接得上：现场生成的话取得到、播得出，
 * 停顿时先问一句「说完没有」，插话打断，刷新后接着面。
 *
 * 假麦克风每 5 秒里说 2.5 秒、安静 2.5 秒，一直循环，不管面试官在不在说话。所以候选人「说话」的那一段
 * 常常正好碰上面试官在说，这就是插话，现场追问的那一句往往刚开口就被打断。要看面试官把话说完，
 * 得让候选人等面试官说完再开口（见 politeCandidate）。
 */

/** 替身在每道题上追问的那一句（crates/interview-room/src/stand_in.rs）。 */
const PROBE = "你本人具体负责的是哪一部分？";

/** 候选人停顿时页面先问服务端「说完没有」，把每次的结论记下来。 */
function watchPauseChecks(page: Page): string[] {
  const verdicts: string[] = [];
  page.on("response", (response) => {
    if (!response.url().endsWith("/api/v1/room/pause-check")) return;
    response.json().then(
      (body: { verdict?: string }) => verdicts.push(body.verdict ?? "failed"),
      () => verdicts.push("failed"),
    );
  });
  return verdicts;
}

/** 候选人插话时，页面上报面试官的话断在了第几毫秒。 */
function watchBargeIns(page: Page): number[] {
  const cuts: number[] = [];
  page.on("request", (request) => {
    if (!request.url().endsWith("/api/v1/room/advance")) return;
    const body = request.postDataJSON() as { interrupted_played_ms?: number | null } | null;
    if (typeof body?.interrupted_played_ms === "number") cuts.push(body.interrupted_played_ms);
  });
  return cuts;
}

/**
 * 让候选人等面试官说完再开口：面试官在说话或者在想的时候把麦克风关成静音，轮到候选人了再打开。
 * 顺带记下面试中页面上依次显示过面试官的哪些话（一句只显示一两秒，靠轮询等不一定赶得上）。
 */
async function politeCandidate(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const scope = window as unknown as { __captions: string[] };
    scope.__captions = [];
    const streams: MediaStream[] = [];
    const original = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = async (constraints) => {
      const stream = await original(constraints);
      streams.push(stream);
      return stream;
    };
    setInterval(() => {
      // 面试官头像下面的小标签：「正在提问」「正在听」「思考中」。不在面试页上（设备检测）时没有它
      const presence = document.querySelector(".oil-presence__label");
      const mayTalk = !presence || presence.textContent === "正在听";
      for (const stream of streams) {
        for (const track of stream.getAudioTracks()) track.enabled = mayTalk;
      }
      const caption = document.querySelector(".app-scene__caption")?.textContent;
      if (caption && scope.__captions.at(-1) !== caption) scope.__captions.push(caption);
    }, 30);
  });
}

const shownCaptions = (page: Page): Promise<string[]> =>
  page.evaluate(() => (window as unknown as { __captions: string[] }).__captions);

/** 面试官依次说了哪几句：过渡语（ack.N，每次不一定是同一句）不算，现场的一段话拆成几句的只算一次。 */
function spokenInOrder(keys: (string | null)[]): (string | null)[] {
  return keys
    .filter((key) => !key?.startsWith("ack."))
    .filter((key, index, all) => key !== all[index - 1]);
}

test("实时对话：面试官在每道题上现场追问一句，候选人答完进入下一题，一直走到结束", async ({
  page,
}) => {
  const verdicts = watchPauseChecks(page);
  const cuts = watchBargeIns(page);
  await politeCandidate(page);
  const interview = await enterInterview(page, { realtime: true });
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 110_000 });

  // 现场生成的追问取到了、播了、显示在页面上
  const captions = await shownCaptions(page);
  expect(captions.some((caption) => caption.includes(PROBE))).toBe(true);

  const state = await expectRecordingsReady(interview, 1);
  await test.info().attach("面试经过", { body: timeline(state), contentType: "text/plain" });
  expect(state.status).toBe("completed");
  expect(state.end_reason).toBe("completed");
  expect(spokenInOrder(spokenKeys(state))).toEqual([
    "opening",
    "q1",
    "q1.p1",
    "q2",
    "q2.p1",
    "q3",
    "q3.p1",
    "candidate_questions",
    "closing",
  ]);

  // 候选人的每一段话都记在它回答的那一句下面
  const answers = state.turns.filter((turn) => turn.speaker === "candidate");
  expect(answers.map((turn) => turn.utterance_key)).toEqual([
    "q1",
    "q1.p1",
    "q2",
    "q2.p1",
    "q3",
    "q3.p1",
    "candidate_questions",
  ]);
  // 回答主问题的那几段从录像里转写出了文字。回答追问的那几段这里不查：假的识别后端一次只有一句话
  // 的字可分，录像里面试官自己的声音也算声音，追问说得久，字在候选人开口之前就分完了（#401）。
  // 真的识别后端没有这个限制
  for (const key of ["q1", "q2", "q3"]) {
    expect(
      answers.find((turn) => turn.utterance_key === key)?.has_text,
      `回答 ${key} 没有文字`,
    ).toBe(true);
  }

  // 停顿时先问了服务端，服务端说「说完了」，不用等满静音时长
  expect(verdicts).toContain("done");
  expect(verdicts).not.toContain("failed");
  // 候选人没有在面试官说话时出声，没有哪一句被打断
  expect(cuts).toEqual([]);
});

test("实时对话里候选人插话，中途还刷新了页面：面试官停下来听，刷新后接着对话，两段录像都完整", async ({
  page,
}) => {
  const cuts = watchBargeIns(page);
  const interview = await enterInterview(page, { realtime: true });
  // 等面试官在第一题上现场追问过了再刷新
  await expect.poll(() => spokenKeys(roomState(interview)), { timeout: 30_000 }).toContain("q1.p1");
  // 面试官说话时候选人出声，算插话：页面停下面试官的话，把断在哪报给服务端
  expect(cuts.length).toBeGreaterThan(0);
  await page.reload();

  await expect(button(page, "检查设备，继续面试")).toBeVisible();
  await passPreflight(page, { resuming: true });
  await button(page, "继续面试").click();
  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 100_000 });

  const state = await expectRecordingsReady(interview, 2);
  await test.info().attach("面试经过", { body: timeline(state), contentType: "text/plain" });
  expect(state.end_reason).toBe("completed");
  const spoken = spokenKeys(state);
  // 刷新之后还是实时对话：后面的题上照样有现场的追问。主问题本身不查：候选人在过渡语上插话时，
  // 排在后面还没播的主问题就不播了，真模型这时会把题重新问一遍，替身不会
  for (const key of ["q2.p1", "q3.p1"]) {
    expect(spoken, `面试官没说 ${key}`).toContain(key);
  }
  expect(spoken.at(-1)).toBe("closing");
  expect(state.events.filter((event) => event.kind === "interview_started")).toEqual([
    expect.objectContaining({ payload: expect.objectContaining({ resumed: false }) }),
    expect.objectContaining({ payload: expect.objectContaining({ resumed: true }) }),
  ]);
});
