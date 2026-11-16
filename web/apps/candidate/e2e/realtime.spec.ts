import { expect, type Page, test } from "@playwright/test";
import {
  button,
  enterInterview,
  eventKinds,
  expectRecordingsReady,
  open,
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
  // 每一段都从录像里转写出了文字，回答现场追问的那几段也一样：录像里面试官自己的声音也算声音，
  // 假的识别后端的字分完以后会从头再分（#401），不然追问说得久，轮到候选人时就没字了
  for (const answer of answers) {
    expect(answer.has_text, `回答 ${answer.utterance_key} 没有文字`).toBe(true);
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

test("实时对话里网络断了几秒、接口出了几次错：面试官晚一点接话，一句不多一句不少，面试照常走完", async ({
  page,
  context,
}) => {
  const verdicts = watchPauseChecks(page);
  await politeCandidate(page);
  // 头两次问「说完没有」都出错；报第二题的回答时头两次服务不可用，像后端正在重启
  let failedChecks = 0;
  await page.route("**/api/v1/room/pause-check", async (route) => {
    if (failedChecks >= 2) return route.continue();
    failedChecks += 1;
    await route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({ error: { code: "internal", message: "服务出错了" } }),
    });
  });
  let failedAdvances = 0;
  await page.route("**/api/v1/room/advance", async (route) => {
    const body = route.request().postDataJSON() as { utterance_key?: string } | null;
    if (body?.utterance_key !== "q2" || failedAdvances >= 2) return route.continue();
    failedAdvances += 1;
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: JSON.stringify({ error: { code: "unavailable", message: "服务暂时不可用" } }),
    });
  });

  const interview = await enterInterview(page, { realtime: true });
  // 面试官在第一题上追问的时候断网，候选人回答追问的那一段正好报不上去
  await expect.poll(() => spokenKeys(roomState(interview)), { timeout: 30_000 }).toContain("q1.p1");
  await context.setOffline(true);
  await expect(page.getByText("网络不稳定")).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(6000);
  await context.setOffline(false);

  await expect(page.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 110_000 });
  const state = await expectRecordingsReady(interview, 1);
  await test.info().attach("面试经过", { body: timeline(state), contentType: "text/plain" });
  expect(state.end_reason).toBe("completed");
  expect(failedChecks).toBe(2);
  expect(failedAdvances).toBe(2);
  // 问不到「说完没有」时页面按静音时长自己判断，没有卡住
  expect(verdicts.filter((verdict) => verdict === "failed")).toHaveLength(2);

  // 重试没有让哪一句多说一遍，也没有漏掉哪一句
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
  // 断网时说的那一段也在录像里，网络恢复后传上去，照样转写出了文字
  for (const answer of answers) {
    expect(answer.has_text, `回答 ${answer.utterance_key} 没有文字`).toBe(true);
  }
  expect(eventKinds(state)).toEqual(expect.arrayContaining(["network_offline", "network_online"]));
});

test("实时对话到一半换到另一个页面：那边接着对话，这边停下来不再问服务端，一句不多一句不少", async ({
  page,
  browser,
}) => {
  await politeCandidate(page);
  const interview = await enterInterview(page, { realtime: true });

  // 另一个页面（像是换了一台设备）打开同一条链接，检查好设备，先不点「继续面试」
  const context = await browser.newContext();
  const other = await context.newPage();
  await politeCandidate(other);
  await open(other, interview.token);
  await passPreflight(other, { resuming: true });
  // 等这边的面试官现场追问过了再换过去
  await expect.poll(() => spokenKeys(roomState(interview)), { timeout: 30_000 }).toContain("q1.p1");
  await button(other, "继续面试").click();
  await expect(other.getByText("录制中")).toBeVisible();

  // 这边几秒之内停下来；停下来以后不再向服务端报回答、问「说完没有」
  await expect(page.getByRole("heading", { name: "面试已经在另一个页面里继续了" })).toBeVisible();
  await expect(page.getByText("录制中")).toBeHidden();
  const late: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.endsWith("/room/advance") || path.endsWith("/room/pause-check")) late.push(path);
  });

  await expect(other.getByText("面试完成，谢谢你")).toBeVisible({ timeout: 110_000 });
  expect(late).toEqual([]);

  const state = await expectRecordingsReady(interview, 2);
  await test.info().attach("面试经过", { body: timeline(state), contentType: "text/plain" });
  expect(state.end_reason).toBe("completed");
  expect(eventKinds(state).filter((kind) => kind === "page_taken_over")).toHaveLength(1);
  // 换页面时正在说的那一句在那边重说一遍，别的不重复、不漏
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
  for (const answer of answers) {
    expect(answer.has_text, `回答 ${answer.utterance_key} 没有文字`).toBe(true);
  }
  await context.close();
});
