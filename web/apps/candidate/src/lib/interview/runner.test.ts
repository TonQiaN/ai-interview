import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoomApiError } from "../api/room-api";
import type { AdvanceRequest, NextStep } from "../api/room-types";
import type { Playback } from "../media/audio-graph";
import { InterviewRunner, levelFromDb, readingTimeMs, turnConfigFor } from "./runner";

/** 假的播放器：每段语音的时长由「解码结果」决定，到时间自己播完。 */
class FakeAudio {
  played: number[] = [];
  decode = async (data: ArrayBuffer): Promise<number> => {
    if (data.byteLength === 0) throw new Error("解码失败");
    return data.byteLength; // 字节数当作毫秒数
  };
  play = (durationMs: number): Playback => {
    this.played.push(durationMs);
    let settle: (result: "ended" | "stopped") => void = () => {};
    const done = new Promise<"ended" | "stopped">((resolve) => {
      settle = resolve;
    });
    const timer = setTimeout(() => settle("ended"), durationMs);
    return {
      durationMs,
      done,
      stop: () => {
        clearTimeout(timer);
        settle("stopped");
      },
    };
  };
}

function say(key: string, options: { expectsAnswer?: boolean; audioMs?: number | null; index?: number } = {}): NextStep {
  const { expectsAnswer = true, audioMs = 2_000, index = 1 } = options;
  return {
    action: "say",
    utterance: {
      key,
      kind: expectsAnswer ? "question" : "opening",
      text: `这是 ${key} 的文字`,
      audio_url: audioMs === null ? null : `/audio/${key}?ms=${audioMs}`,
      audio_duration_ms: audioMs,
    },
    expects_answer: expectsAnswer,
    answer: expectsAnswer ? { silence_sec: 5, max_sec: 60, min_sec: 3 } : null,
    progress: { index, total: 3, elapsed_ms: 0, expected_total_ms: 600_000 },
  };
}

const finish: NextStep = { action: "finish", reason: "completed" };

class FakeApi {
  requests: AdvanceRequest[] = [];
  /** advance 接下来要抛的错，用完一个少一个。 */
  failures: unknown[] = [];
  constructor(private readonly steps: NextStep[]) {}

  advance = async (request: AdvanceRequest): Promise<NextStep> => {
    const failure = this.failures.shift();
    if (failure) throw failure;
    this.requests.push(request);
    return this.steps.shift() ?? finish;
  };

  fetchAudio = async (url: string): Promise<ArrayBuffer> => {
    const ms = Number(new URL(url, "http://x").searchParams.get("ms"));
    if (Number.isNaN(ms)) throw new Error("取不到");
    return new ArrayBuffer(ms);
  };
}

function setup(steps: NextStep[]) {
  const api = new FakeApi(steps);
  const audio = new FakeAudio();
  const runner = new InterviewRunner({ api, audio, now: () => Date.now(), random: () => 0.5 });
  const phases: string[] = [];
  runner.subscribe((snapshot) => {
    if (phases.at(-1) !== snapshot.phase) phases.push(snapshot.phase);
  });
  return { api, audio, runner, phases };
}

/**
 * 喂一段音量：loud 为真是说话，否则是安静。
 * 真人说话字与字之间有短暂的停顿，这里每 400 毫秒留 60 毫秒的空隙；
 * 没有空隙的持续响声会被当成环境噪声。
 */
async function feed(runner: InterviewRunner<number>, ms: number, loud: boolean) {
  for (let elapsed = 0; elapsed < ms; elapsed += 20) {
    const gap = elapsed % 400 >= 340;
    runner.handleLevel({ rms: loud && !gap ? 0.2 : 0.0005, frameMs: 20 });
    await vi.advanceTimersByTimeAsync(20);
  }
}

/** 面试官说话期间麦克风是安静的。 */
const quiet = (runner: InterviewRunner<number>, ms: number) => feed(runner, ms, false);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("逐题面试编排", () => {
  it("播题、听回答、静音后上报，一直到结束", async () => {
    const { api, audio, runner, phases } = setup([say("q2", { index: 2 }), finish]);
    const result = runner.run(say("q1"));

    await vi.advanceTimersByTimeAsync(0);
    expect(runner.getSnapshot().phase).toBe("asking");
    await quiet(runner, 2_000);
    expect(runner.getSnapshot().phase).toBe("listening");

    await feed(runner, 1_000, false); // 想了一秒
    await feed(runner, 4_000, true); // 说了四秒
    expect(runner.getSnapshot().speaking).toBe(true);
    await feed(runner, 3_000, false);
    // 静音三秒：还剩两秒多，倒计时出来了
    expect(runner.getSnapshot().turn?.countdownVisible).toBe(true);
    await feed(runner, 3_000, false);

    expect(api.requests).toHaveLength(1);
    const first = api.requests[0] as AdvanceRequest;
    expect(first).toMatchObject({ utterance_key: "q1", playback_start_ms: 0, playback_end_ms: 2_000, end_reason: "silence" });
    // 第 3 秒开口（检测要 150 毫秒），第 7 秒停下（检测要 400 毫秒）
    expect(first.answer_start_ms).toBeGreaterThanOrEqual(3_000);
    expect(first.answer_start_ms).toBeLessThan(3_300);
    expect(first.answer_end_ms).toBeGreaterThanOrEqual(7_000);
    expect(first.answer_end_ms).toBeLessThan(7_600);

    // 第二题：点「我答完了」
    expect(runner.getSnapshot()).toMatchObject({ phase: "asking", utterance: { key: "q2" }, progress: { index: 2 } });
    await quiet(runner, 2_000);
    await feed(runner, 1_000, true);
    runner.done();
    await vi.advanceTimersByTimeAsync(0);

    expect(await result).toBe("completed");
    expect(api.requests[1]).toMatchObject({ utterance_key: "q2", end_reason: "button" });
    expect(audio.played).toEqual([2_000, 2_000]);
    expect(runner.getSnapshot()).toMatchObject({ phase: "finished", finishReason: "completed" });
    expect(phases).toEqual([
      "preparing", "asking", "listening", "submitting",
      "preparing", "asking", "listening", "submitting",
      "finished",
    ]);
  });

  it("不需要回答的话播完就直接进下一步", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("opening", { expectsAnswer: false, audioMs: 3_000 }));
    await vi.advanceTimersByTimeAsync(3_000);
    expect(await result).toBe("completed");
    expect(api.requests[0]).toEqual({
      utterance_key: "opening",
      playback_start_ms: 0,
      playback_end_ms: 3_000,
      answer_start_ms: null,
      answer_end_ms: null,
      end_reason: null,
      replays: 0,
    });
  });

  it("面试官说话时候选人出声不算作答", async () => {
    const { runner } = setup([finish]);
    void runner.run(say("q1", { audioMs: 4_000 }));
    await vi.advanceTimersByTimeAsync(0);
    await feed(runner, 4_000, true); // 整段播放期间都有声音（比如外放的回声）
    await feed(runner, 200, false);
    const snapshot = runner.getSnapshot();
    expect(snapshot.phase).toBe("listening");
    expect(snapshot.speaking).toBe(false);
    expect(snapshot.turn?.silenceRemainingMs).toBeNull(); // 还没开过口，不计静音
    runner.stop();
  });

  it("一直不开口：先提示，再自动进下一题", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 13_000, false);
    expect(runner.getSnapshot().turn?.noSpeechPromptVisible).toBe(true);
    await feed(runner, 18_000, false);
    expect(await result).toBe("completed");
    expect(api.requests[0]).toMatchObject({ end_reason: "no_speech", answer_start_ms: null, answer_end_ms: null });
  });

  it("到了最长作答时间就结束这一题", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 61_000, true);
    expect(await result).toBe("completed");
    expect(api.requests[0]).toMatchObject({ end_reason: "timeout", answer_end_ms: 62_000 });
  });

  it("开口前可以重听题目，上报里记下重听次数", async () => {
    const { api, audio, runner } = setup([finish]);
    const result = runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 1_000, false);
    expect(runner.getSnapshot().canReplay).toBe(true);

    runner.replay();
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.getSnapshot().phase).toBe("asking");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(runner.getSnapshot().phase).toBe("listening");

    await feed(runner, 1_000, true);
    expect(runner.getSnapshot().canReplay).toBe(false); // 开口之后不能再重听
    runner.done();
    await vi.advanceTimersByTimeAsync(0);
    expect(await result).toBe("completed");
    expect(audio.played).toEqual([2_000, 2_000]);
    // 播放的起点是第一次开始播，终点是最后一次播完
    expect(api.requests[0]).toMatchObject({ replays: 1, playback_start_ms: 0, playback_end_ms: 5_000 });
  });

  it("倒计时期间点「继续回答」会重新计静音", async () => {
    const { api, runner } = setup([finish]);
    void runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 2_000, true);
    await feed(runner, 4_500, false);
    expect(runner.getSnapshot().turn?.countdownVisible).toBe(true);
    runner.continueAnswer();
    await feed(runner, 4_000, false);
    expect(api.requests).toHaveLength(0);
    await feed(runner, 1_500, false);
    expect(api.requests).toHaveLength(1);
  });

  it("语音取不到时改成显示文字，等候选人点「开始回答」", async () => {
    const { api, audio, runner } = setup([finish]);
    const result = runner.run(say("q1", { audioMs: null }));
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.getSnapshot()).toMatchObject({ phase: "reading", canReplay: false });

    await vi.advanceTimersByTimeAsync(6_000); // 读题
    runner.ready();
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.getSnapshot().phase).toBe("listening");
    await feed(runner, 1_000, true);
    runner.done();
    await vi.advanceTimersByTimeAsync(0);

    expect(await result).toBe("completed");
    expect(audio.played).toEqual([]);
    expect(api.requests[0]).toMatchObject({ playback_start_ms: 0, playback_end_ms: 6_000, end_reason: "button" });
  });

  it("语音解码失败、又不需要回答：文字停留一会儿再继续", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("closing", { expectsAnswer: false, audioMs: 0 }));
    await vi.advanceTimersByTimeAsync(0);
    expect(runner.getSnapshot().phase).toBe("reading");
    await vi.advanceTimersByTimeAsync(3_000);
    expect(await result).toBe("completed");
    expect(api.requests[0]).toMatchObject({ utterance_key: "closing", playback_end_ms: 3_000 });
  });

  it("语音太慢时不干等，超时后显示文字", async () => {
    const api = new FakeApi([finish]);
    api.fetchAudio = () => new Promise(() => {}); // 永远不返回
    const runner = new InterviewRunner({ api, audio: new FakeAudio(), now: () => Date.now(), audioTimeoutMs: 5_000 });
    void runner.run(say("q1"));
    await vi.advanceTimersByTimeAsync(4_900);
    expect(runner.getSnapshot().phase).toBe("preparing");
    await vi.advanceTimersByTimeAsync(200);
    expect(runner.getSnapshot().phase).toBe("reading");
    runner.stop();
  });

  it("提前取好后面的语音", async () => {
    const api = new FakeApi([finish]);
    const fetched: string[] = [];
    const original = api.fetchAudio;
    api.fetchAudio = (url) => {
      fetched.push(url);
      return original(url);
    };
    const runner = new InterviewRunner({ api, audio: new FakeAudio(), now: () => Date.now() });
    const first = say("q1");
    if (first.action === "say") first.preload_audio_urls = ["/audio/q2?ms=1000", "/audio/q1?ms=2000"];
    void runner.run(first);
    await vi.advanceTimersByTimeAsync(0);
    // q1 自己的语音只取一次
    expect(fetched.sort()).toEqual(["/audio/q1?ms=2000", "/audio/q2?ms=1000"]);
    runner.stop();
  });

  it("上报时断网会一直重试，恢复后继续", async () => {
    const { api, runner } = setup([finish]);
    api.failures = [new RoomApiError("network", "断网"), new TypeError("fetch failed"), new RoomApiError("rate_limited", "太快")];
    const result = runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 500, true);
    runner.done();
    await vi.advanceTimersByTimeAsync(0);

    expect(runner.getSnapshot()).toMatchObject({ phase: "submitting", submitFailures: 1 });
    await vi.advanceTimersByTimeAsync(1_000);
    expect(runner.getSnapshot().submitFailures).toBe(2);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(runner.getSnapshot().submitFailures).toBe(3);
    await vi.advanceTimersByTimeAsync(4_000);

    expect(await result).toBe("completed");
    expect(api.requests).toHaveLength(1);
    expect(runner.getSnapshot().submitFailures).toBe(0);
  });

  it("链接失效这类错误不重试，直接失败", async () => {
    const { api, runner } = setup([finish]);
    api.failures = [new RoomApiError("expired", "链接已过期", 410, "invite_expired")];
    const result = runner.run(say("q1"));
    const settled = result.catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(2_000);
    runner.done();
    await vi.advanceTimersByTimeAsync(0);

    const error = await settled;
    expect(error).toBeInstanceOf(RoomApiError);
    expect(runner.getSnapshot()).toMatchObject({ phase: "failed", error: { kind: "expired" } });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(api.requests).toHaveLength(0);
  });

  it("设备被打断时暂停计时，恢复后接着算", async () => {
    const { api, runner } = setup([finish]);
    void runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 2_000, true);
    await feed(runner, 1_000, false);

    runner.pause();
    expect(runner.getSnapshot().paused).toBe(true);
    await feed(runner, 30_000, true); // 暂停期间的声音不算
    expect(api.requests).toHaveLength(0);

    runner.resume();
    expect(runner.getSnapshot().paused).toBe(false);
    await feed(runner, 4_900, false);
    expect(api.requests).toHaveLength(0);
    await feed(runner, 300, false);
    expect(api.requests).toHaveLength(1);
  });

  it("stop 之后不再播放也不再上报", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("q1"));
    await vi.advanceTimersByTimeAsync(1_000);
    runner.stop();
    expect(await result).toBeNull();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(api.requests).toHaveLength(0);
  });

  it("把状态机的变化报给外面，用来记过程事件", async () => {
    const api = new FakeApi([finish]);
    const seen: string[] = [];
    const runner = new InterviewRunner({
      api,
      audio: new FakeAudio(),
      now: () => Date.now(),
      onTurnEvent: (event) => seen.push(event.type),
    });
    void runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 1_000, true);
    await feed(runner, 6_000, false);
    expect(seen).toEqual(["question_ended", "speech_start", "speech_end"]);
  });
});

describe("作答设置", () => {
  it("把秒换成毫秒，并让各项时间互相合理", () => {
    expect(turnConfigFor({ silence_sec: 2, max_sec: 20, min_sec: 1 })).toMatchObject({
      silenceMs: 2_000,
      countdownMs: 2_000,
      minAnswerMs: 1_000,
      maxAnswerMs: 20_000,
      maxAnswerWarningMs: 5_000,
      noSpeechTimeoutMs: 20_000,
    });
    expect(turnConfigFor({ silence_sec: 10, max_sec: 300, min_sec: 3 })).toMatchObject({
      silenceMs: 10_000,
      countdownMs: 3_000,
      maxAnswerWarningMs: 15_000,
      noSpeechTimeoutMs: 30_000,
    });
  });

  it("文字停留时间和字数有关，有上下限", () => {
    expect(readingTimeMs("好")).toBe(3_000);
    expect(readingTimeMs("字".repeat(40))).toBe(8_000);
    expect(readingTimeMs("字".repeat(500))).toBe(20_000);
  });

  it("音量条读数在 0 到 1 之间", () => {
    expect(levelFromDb(-100)).toBe(0);
    expect(levelFromDb(-35)).toBeCloseTo(0.5);
    expect(levelFromDb(0)).toBe(1);
  });
});
