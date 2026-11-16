import { describe, expect, it } from "vitest";
import {
  DEFAULT_TURN_CONFIG,
  reduceTurn,
  startTurn,
  type TurnConfig,
  type TurnEvent,
  type TurnState,
  viewTurn,
} from "./turn-machine";

const config: TurnConfig = { ...DEFAULT_TURN_CONFIG };

function run(events: TurnEvent[], initial: TurnState = startTurn(0)): TurnState {
  return events.reduce((state, event) => reduceTurn(state, event, config), initial);
}

/** 题目在第 10 秒播完，进入听作答。 */
const listening = (): TurnState => run([{ type: "question_ended", at: 10_000 }]);

describe("逐题状态机", () => {
  it("题目播完后进入听作答", () => {
    const state = listening();
    expect(state).toMatchObject({
      phase: "listening",
      listenStartedAt: 10_000,
      firstSpeechAt: null,
    });
  });

  it("播放题目期间的声音不算作答", () => {
    const state = run([
      { type: "speech_start", at: 2_000 },
      { type: "speech_end", at: 3_000 },
      { type: "tick", at: 9_000 },
    ]);
    expect(state.phase).toBe("asking");
  });

  it("面试官还在说、候选人插话：直接进入听作答，开口时刻就是插话的时刻", () => {
    let state = run([{ type: "barge_in", at: 2_450, speechAt: 2_000 }]);
    expect(state).toMatchObject({
      phase: "listening",
      listenStartedAt: 2_000,
      firstSpeechAt: 2_000,
      speaking: true,
      silenceSince: null,
    });
    state = run(
      [
        { type: "speech_end", at: 5_400, quietMs: 400 },
        { type: "tick", at: 5_000 + config.silenceMs },
      ],
      state,
    );
    expect(state).toMatchObject({
      phase: "ended",
      reason: "silence",
      firstSpeechAt: 2_000,
      lastSpeechEndAt: 5_400,
    });
  });

  it("没到最短作答时间时，静音够久也不结束", () => {
    const short: TurnConfig = { ...config, silenceMs: 1_000, minAnswerMs: 4_000 };
    let state = listening();
    state = reduceTurn(state, { type: "speech_start", at: 10_200 }, short);
    state = reduceTurn(state, { type: "speech_end", at: 10_600 }, short);
    state = reduceTurn(state, { type: "tick", at: 12_000 }, short);
    expect(state.phase).toBe("listening");
    // 倒计时显示的是离真正结束还有多久，不是离静音阈值还有多久
    expect(viewTurn(state, 12_000, short).silenceRemainingMs).toBe(2_000);
    state = reduceTurn(state, { type: "tick", at: 14_000 }, short);
    expect(state).toMatchObject({ phase: "ended", reason: "silence" });
  });

  it("静音从声音真正停下的那一刻算起，回答的结束时刻还是听出停止的那一刻", () => {
    // VAD 等声音停了 400 毫秒才报「停止说话」
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 40_000, quietMs: 400 },
        { type: "tick", at: 44_500 },
      ],
      listening(),
    );
    expect(state).toMatchObject({ phase: "listening", silenceSince: 39_600 });
    expect(viewTurn(state, 44_500, config).silenceRemainingMs).toBe(100);

    const ended = reduceTurn(state, { type: "tick", at: 44_600 }, config);
    expect(ended).toMatchObject({
      phase: "ended",
      reason: "silence",
      lastSpeechEndAt: 40_000,
      endedAt: 44_600,
    });
  });

  it("报上来的安静时长再离谱，静音也不会从开口之前算起", () => {
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 12_300, quietMs: 60_000 },
      ],
      listening(),
    );
    expect(state).toMatchObject({ phase: "listening", silenceSince: 12_000 });
  });

  it("开口后连续静音达到设定时长算答完", () => {
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 40_000 },
        { type: "tick", at: 44_900 },
      ],
      listening(),
    );
    expect(state.phase).toBe("listening");

    const ended = reduceTurn(state, { type: "tick", at: 45_000 }, config);
    expect(ended).toMatchObject({
      phase: "ended",
      reason: "silence",
      firstSpeechAt: 12_000,
      lastSpeechEndAt: 40_000,
      endedAt: 45_000,
    });
  });

  it("思考停顿后继续说，静音重新计时", () => {
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 20_000 },
        { type: "tick", at: 24_000 },
        { type: "speech_start", at: 24_500 },
        { type: "tick", at: 30_000 },
        { type: "speech_end", at: 35_000 },
        { type: "tick", at: 39_000 },
      ],
      listening(),
    );
    expect(state).toMatchObject({ phase: "listening", silenceSince: 35_000 });
  });

  it("倒计时只在最后几秒显示，点「继续回答」后重新计时", () => {
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 20_000 },
      ],
      listening(),
    );
    expect(viewTurn(state, 21_000, config)).toMatchObject({
      countdownVisible: false,
      silenceRemainingMs: 4_000,
    });
    expect(viewTurn(state, 22_500, config)).toMatchObject({
      countdownVisible: true,
      silenceRemainingMs: 2_500,
    });

    const continued = reduceTurn(state, { type: "user_continue", at: 23_000 }, config);
    expect(viewTurn(continued, 23_000, config)).toMatchObject({
      countdownVisible: false,
      silenceRemainingMs: 5_000,
    });
    expect(reduceTurn(continued, { type: "tick", at: 27_000 }, config).phase).toBe("listening");
    expect(reduceTurn(continued, { type: "tick", at: 28_000 }, config)).toMatchObject({
      reason: "silence",
    });
  });

  it("一直不开口：先提示，再超时进入下一题", () => {
    let state = run([{ type: "tick", at: 21_000 }], listening());
    expect(viewTurn(state, 21_000, config).noSpeechPromptVisible).toBe(false);

    state = reduceTurn(state, { type: "tick", at: 22_000 }, config);
    expect(viewTurn(state, 22_000, config).noSpeechPromptVisible).toBe(true);

    state = reduceTurn(state, { type: "tick", at: 40_000 }, config);
    expect(state).toMatchObject({
      phase: "ended",
      reason: "no_speech",
      firstSpeechAt: null,
      lastSpeechEndAt: null,
    });
  });

  it("提示之后开口，提示消失，不再按没开口处理", () => {
    const state = run(
      [
        { type: "tick", at: 23_000 },
        { type: "speech_start", at: 25_000 },
        { type: "tick", at: 60_000 },
      ],
      listening(),
    );
    expect(state.phase).toBe("listening");
    expect(viewTurn(state, 60_000, config).noSpeechPromptVisible).toBe(false);
  });

  it("到了最长作答时间直接结束，哪怕还在说", () => {
    const state = run(
      [
        { type: "speech_start", at: 11_000 },
        { type: "tick", at: 189_999 },
      ],
      listening(),
    );
    expect(state.phase).toBe("listening");
    expect(viewTurn(state, 176_000, config)).toMatchObject({
      maxWarningVisible: true,
      answerRemainingMs: 14_000,
    });

    const ended = reduceTurn(state, { type: "tick", at: 190_000 }, config);
    expect(ended).toMatchObject({
      phase: "ended",
      reason: "max_duration",
      lastSpeechEndAt: 190_000,
    });
  });

  it("候选人点「我答完了」立即结束", () => {
    const state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "user_done", at: 30_000 },
      ],
      listening(),
    );
    expect(state).toMatchObject({ phase: "ended", reason: "manual", lastSpeechEndAt: 30_000 });
  });

  it("没开口之前可以重听，次数有限", () => {
    let state = listening();
    expect(viewTurn(state, 11_000, config).canReplay).toBe(true);

    state = reduceTurn(state, { type: "user_replay", at: 11_000 }, config);
    expect(state).toMatchObject({ phase: "asking", replaysUsed: 1 });
    state = run(
      [
        { type: "question_ended", at: 20_000 },
        { type: "user_replay", at: 21_000 },
        { type: "question_ended", at: 30_000 },
      ],
      state,
    );
    expect(state).toMatchObject({ phase: "listening", replaysUsed: 2, listenStartedAt: 30_000 });
    expect(viewTurn(state, 31_000, config).canReplay).toBe(false);
    expect(reduceTurn(state, { type: "user_replay", at: 31_000 }, config)).toBe(state);
  });

  it("开口之后不能重听", () => {
    const state = run([{ type: "speech_start", at: 12_000 }], listening());
    expect(viewTurn(state, 13_000, config).canReplay).toBe(false);
    expect(reduceTurn(state, { type: "user_replay", at: 13_000 }, config)).toBe(state);
  });

  it("题目语音放不出来：等候选人点「开始回答」再开始计时", () => {
    let state = run([{ type: "question_failed", at: 1_000 }]);
    expect(state.phase).toBe("reading");
    state = reduceTurn(state, { type: "tick", at: 120_000 }, config);
    expect(state.phase).toBe("reading");
    state = reduceTurn(state, { type: "user_ready", at: 125_000 }, config);
    expect(state).toMatchObject({ phase: "listening", listenStartedAt: 125_000 });
  });

  it("暂停期间不计时，恢复后继续", () => {
    let state = run(
      [
        { type: "speech_start", at: 12_000 },
        { type: "speech_end", at: 20_000 },
        { type: "pause", at: 22_000 },
        // 暂停了一分钟，期间的事件都不生效
        { type: "tick", at: 60_000 },
        { type: "speech_start", at: 61_000 },
        { type: "user_done", at: 62_000 },
      ],
      listening(),
    );
    expect(state).toMatchObject({ phase: "listening", pausedAt: 22_000 });
    expect(viewTurn(state, 60_000, config).silenceRemainingMs).toBe(3_000);

    state = reduceTurn(state, { type: "resume", at: 82_000 }, config);
    // 恢复后静音重新计，已用的作答时间不包含暂停的那一分钟
    expect(state).toMatchObject({ pausedAt: null, silenceSince: 82_000, listenStartedAt: 70_000 });
    expect(reduceTurn(state, { type: "tick", at: 86_999 }, config).phase).toBe("listening");
    expect(reduceTurn(state, { type: "tick", at: 87_000 }, config)).toMatchObject({
      reason: "silence",
    });
  });

  it("结束后不再响应任何事件", () => {
    const ended = run([{ type: "user_done", at: 15_000 }], listening());
    expect(reduceTurn(ended, { type: "speech_start", at: 16_000 }, config)).toBe(ended);
    expect(reduceTurn(ended, { type: "tick", at: 999_000 }, config)).toBe(ended);
  });
});

describe("停顿时服务端先判断说完没有", () => {
  const checked: TurnConfig = {
    ...DEFAULT_TURN_CONFIG,
    silenceMs: 2_000,
    countdownMs: 2_000,
    minAnswerMs: 1_000,
    pauseCheck: { afterMs: 500, doneSilenceMs: 1_000, waitSilenceMs: 3_500, maxChecks: 8 },
  };
  const play = (events: TurnEvent[], settings: TurnConfig = checked): TurnState =>
    events.reduce((state, event) => reduceTurn(state, event, settings), startTurn(0));
  /** 第 10 秒题目播完，第 11 秒开口，13.6 秒声音停下，14 秒听出来停了。 */
  const paused: TurnEvent[] = [
    { type: "question_ended", at: 10_000 },
    { type: "speech_start", at: 11_000 },
    { type: "speech_end", at: 14_000, quietMs: 400 },
  ];
  const verdict = (at: number, value: "done" | "wait", silenceSince = 13_600): TurnEvent => ({
    type: "pause_verdict",
    at,
    verdict: value,
    silenceSince,
  });

  it("说完了：静音满短的那个时长就算答完，不用等满", () => {
    const judged = play([...paused, verdict(14_300, "done")]);
    expect(judged.phase).toBe("listening");
    expect(viewTurn(judged, 14_300, checked).silenceRemainingMs).toBe(300);
    const ended = play([...paused, verdict(14_300, "done"), { type: "tick", at: 14_600 }]);
    expect(ended).toMatchObject({ phase: "ended", reason: "silence", lastSpeechEndAt: 14_000 });
  });

  it("判断回来得晚、短的时长已经过了，当场算答完", () => {
    const ended = play([...paused, verdict(14_800, "done")]);
    expect(ended).toMatchObject({ phase: "ended", reason: "silence", endedAt: 14_800 });
  });

  it("没说完：这次停顿多等一会儿", () => {
    const waiting = play([...paused, verdict(14_300, "wait"), { type: "tick", at: 15_700 }]);
    expect(waiting.phase).toBe("listening");
    expect(viewTurn(waiting, 15_700, checked).silenceRemainingMs).toBe(1_400);
    const ended = play([...paused, verdict(14_300, "wait"), { type: "tick", at: 17_100 }]);
    expect(ended).toMatchObject({ phase: "ended", reason: "silence" });
  });

  it("判断回来时候选人已经接着说了，不算数；下一次停顿照原来的时长", () => {
    const resumed = play([...paused, { type: "speech_start", at: 14_200 }]);
    expect(reduceTurn(resumed, verdict(14_300, "done"), checked)).toBe(resumed);
    const next = play([
      ...paused,
      { type: "speech_start", at: 14_200 },
      verdict(14_300, "done"),
      { type: "speech_end", at: 15_000, quietMs: 400 },
      { type: "tick", at: 15_800 },
    ]);
    expect(next.phase).toBe("listening");
    expect(reduceTurn(next, { type: "tick", at: 16_600 }, checked).phase).toBe("ended");
  });

  it("上一次停顿的判断不会用到这一次停顿上", () => {
    const next = play([
      ...paused,
      verdict(14_100, "done"),
      { type: "speech_start", at: 14_200 },
      { type: "speech_end", at: 15_000, quietMs: 400 },
      { type: "tick", at: 15_800 },
    ]);
    expect(next.phase).toBe("listening");
  });

  it("点了「继续回答」，之前的判断作废", () => {
    const continued = play([
      ...paused,
      verdict(14_300, "done"),
      { type: "user_continue", at: 14_400 },
      { type: "tick", at: 15_500 },
    ]);
    expect(continued.phase).toBe("listening");
    expect(reduceTurn(continued, { type: "tick", at: 16_400 }, checked).phase).toBe("ended");
  });

  it("说完了也要过了最短作答时间才结束", () => {
    const early = play([
      { type: "question_ended", at: 10_000 },
      { type: "speech_start", at: 10_100 },
      { type: "speech_end", at: 10_500, quietMs: 400 },
      verdict(10_700, "done", 10_100),
    ]);
    expect(early.phase).toBe("listening");
    expect(reduceTurn(early, { type: "tick", at: 11_100 }, checked).phase).toBe("ended");
  });

  it("没开这个功能时，判断不改变静音时长", () => {
    const plain: TurnConfig = { ...checked };
    delete plain.pauseCheck;
    const state = play([...paused, verdict(14_300, "done"), { type: "tick", at: 15_000 }], plain);
    expect(state.phase).toBe("listening");
  });
});
