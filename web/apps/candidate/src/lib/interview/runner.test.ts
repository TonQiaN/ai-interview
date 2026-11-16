import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoomApiError } from "../api/room-api";
import type {
  AdvanceRequest,
  AnswerSettings,
  NextStep,
  PauseCheckRequest,
  PauseCheckVerdict,
} from "../api/room-types";
import type { Playback } from "../media/audio-graph";
import { InterviewRunner, levelFromDb, readingTimeMs, turnConfigFor } from "./runner";

/** 假的播放器：每段语音的时长由「解码结果」决定，到时间自己播完。 */
class FakeAudio {
  played: number[] = [];
  decode = async (data: ArrayBuffer): Promise<number> => {
    if (data.byteLength === 0) throw new Error("解码失败");
    return data.byteLength; // 字节数当作毫秒数
  };
  /** 每次播放是从这段语音的哪里开始的。 */
  startedFrom: number[] = [];
  play = (durationMs: number, fromMs = 0): Playback => {
    this.played.push(durationMs);
    this.startedFrom.push(fromMs);
    let settle: (result: "ended" | "stopped") => void = () => {};
    const done = new Promise<"ended" | "stopped">((resolve) => {
      settle = resolve;
    });
    const startedAt = Date.now();
    let endedAtMs: number | null = null;
    const positionMs = () => endedAtMs ?? Math.min(durationMs, fromMs + Date.now() - startedAt);
    const timer = setTimeout(() => {
      endedAtMs = durationMs;
      settle("ended");
    }, durationMs - fromMs);
    return {
      durationMs,
      done,
      stop: () => {
        endedAtMs ??= positionMs();
        clearTimeout(timer);
        settle("stopped");
      },
      positionMs,
    };
  };
}

let nextSeq = 0;

function say(
  key: string,
  options: { expectsAnswer?: boolean; audioMs?: number | null; index?: number } = {},
): NextStep {
  const { expectsAnswer = true, audioMs = 2_000, index = 1 } = options;
  nextSeq += 1;
  return {
    action: "say",
    seq: nextSeq,
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
    preload_audio_urls: [],
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
    expect(first).toMatchObject({
      utterance_key: "q1",
      playback_start_ms: 0,
      playback_end_ms: 2_000,
      end_reason: "silence",
    });
    // 第 3 秒开口（检测要 150 毫秒），第 7 秒停下（检测要 400 毫秒）
    expect(first.answer_start_ms).toBeGreaterThanOrEqual(3_000);
    expect(first.answer_start_ms).toBeLessThan(3_300);
    expect(first.answer_end_ms).toBeGreaterThanOrEqual(7_000);
    expect(first.answer_end_ms).toBeLessThan(7_600);

    // 第二题：点「我答完了」
    expect(runner.getSnapshot()).toMatchObject({
      phase: "asking",
      utterance: { key: "q2" },
      progress: { index: 2 },
    });
    await quiet(runner, 2_000);
    await feed(runner, 1_000, true);
    runner.done();
    await vi.advanceTimersByTimeAsync(0);

    expect(await result).toBe("completed");
    expect(api.requests[1]).toMatchObject({ utterance_key: "q2", end_reason: "button" });
    expect(audio.played).toEqual([2_000, 2_000]);
    expect(runner.getSnapshot()).toMatchObject({ phase: "finished", finishReason: "completed" });
    expect(phases).toEqual([
      "preparing",
      "asking",
      "listening",
      "submitting",
      "preparing",
      "asking",
      "listening",
      "submitting",
      "finished",
    ]);
  });

  it("静音从声音停下的那一刻算起，不把 VAD 确认停止的那一小会儿再等一遍", async () => {
    const { api, runner } = setup([finish]);
    void runner.run(say("q1"));
    await quiet(runner, 2_000);
    await feed(runner, 1_000, false);
    await feed(runner, 4_000, true); // 声音在第 6.94 秒停下，VAD 到第 7.34 秒才确认
    await feed(runner, 4_800, false);
    expect(api.requests).toHaveLength(0);
    // 设定的静音是 5 秒：第 11.94 秒之后的第一次检查就结束，不用等到第 12.34 秒
    await feed(runner, 300, false);
    expect(api.requests).toHaveLength(1);
    const request = api.requests[0] as AdvanceRequest;
    expect(request.end_reason).toBe("silence");
    // 回答的结束时刻还是 VAD 确认停止的那一刻，转写取这一段时不会切掉最后一个字
    expect(request.answer_end_ms).toBeGreaterThanOrEqual(7_300);
  });

  it("同一句话说两次时，各自带回服务端给的序号", async () => {
    // 比如没听到回答，服务端让页面把同一题再问一遍
    const again = say("nudge", { expectsAnswer: false, audioMs: 1_000 });
    const { api, runner } = setup([again, finish]);
    const first = say("nudge", { expectsAnswer: false, audioMs: 1_000 });
    const result = runner.run(first);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(await result).toBe("completed");
    expect(api.requests.map((request) => request.utterance_key)).toEqual(["nudge", "nudge"]);
    expect(api.requests.map((request) => request.seq)).toEqual([
      (first as { seq: number }).seq,
      (again as { seq: number }).seq,
    ]);
    expect(api.requests[0]?.seq).not.toBe(api.requests[1]?.seq);
  });

  it("不需要回答的话播完就直接进下一步", async () => {
    const { api, runner } = setup([finish]);
    const result = runner.run(say("opening", { expectsAnswer: false, audioMs: 3_000 }));
    await vi.advanceTimersByTimeAsync(3_000);
    expect(await result).toBe("completed");
    expect(api.requests[0]).toEqual({
      seq: expect.any(Number),
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
    expect(api.requests[0]).toMatchObject({
      end_reason: "no_speech",
      answer_start_ms: null,
      answer_end_ms: null,
    });
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
    expect(api.requests[0]).toMatchObject({
      replays: 1,
      playback_start_ms: 0,
      playback_end_ms: 5_000,
    });
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
    expect(api.requests[0]).toMatchObject({
      playback_start_ms: 0,
      playback_end_ms: 6_000,
      end_reason: "button",
    });
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
    const runner = new InterviewRunner({
      api,
      audio: new FakeAudio(),
      now: () => Date.now(),
      audioTimeoutMs: 5_000,
    });
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
    if (first.action === "say")
      first.preload_audio_urls = ["/audio/q2?ms=1000", "/audio/q1?ms=2000"];
    void runner.run(first);
    await vi.advanceTimersByTimeAsync(0);
    // q1 自己的语音只取一次
    expect(fetched.sort()).toEqual(["/audio/q1?ms=2000", "/audio/q2?ms=1000"]);
    runner.stop();
  });

  it("上报时断网会一直重试，恢复后继续", async () => {
    const { api, runner } = setup([finish]);
    api.failures = [
      new RoomApiError("network", "断网"),
      new TypeError("fetch failed"),
      new RoomApiError("rate_limited", "太快"),
    ];
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

  it("答完一题、上报之前告诉外面一声；不需要回答的话和重听都不算", async () => {
    const api = new FakeApi([say("q1"), finish]);
    const seen: string[] = [];
    const runner = new InterviewRunner({
      api,
      audio: new FakeAudio(),
      now: () => Date.now(),
      onAnswerEnded: () => seen.push(`答完（已上报 ${api.requests.length} 次）`),
    });
    void runner.run(say("opening", { expectsAnswer: false }));
    await quiet(runner, 2_000); // 开场白播完，直接上报
    expect(api.requests).toHaveLength(1);
    expect(seen).toEqual([]);

    await quiet(runner, 2_000);
    runner.replay(); // 重听一遍，这一题还没答完
    await quiet(runner, 2_000);
    expect(seen).toEqual([]);

    await feed(runner, 1_000, true);
    await feed(runner, 6_000, false); // 静音由计时发现，也要通知到
    expect(seen).toEqual(["答完（已上报 1 次）"]);
    expect(api.requests).toHaveLength(2);
  });
});

describe("停顿时先问服务端说完没有", () => {
  /** 实时对话里的一句：静音两秒算答完，停顿半秒时可以先问。 */
  function live(key: string, maxChecks = 8): NextStep {
    const step = say(key);
    if (step.action !== "say") throw new Error("不会发生");
    return {
      ...step,
      answer: {
        silence_sec: 2,
        max_sec: 60,
        min_sec: 1,
        pause_check: {
          after_ms: 500,
          done_silence_ms: 1_000,
          wait_silence_ms: 3_500,
          max_checks: maxChecks,
        },
      },
    };
  }

  /** 会回答「说完了吗」的假接口：每次问都过 300 毫秒才有结果。 */
  class CheckingApi extends FakeApi {
    checks: PauseCheckRequest[] = [];
    constructor(
      steps: NextStep[],
      private readonly answers: (PauseCheckVerdict | Error)[],
    ) {
      super(steps);
    }

    checkPause = async (request: PauseCheckRequest): Promise<PauseCheckVerdict> => {
      this.checks.push(request);
      await new Promise((resolve) => setTimeout(resolve, 300));
      const answer = this.answers.shift() ?? "unknown";
      if (answer instanceof Error) throw answer;
      return answer;
    };
  }

  function start(first: NextStep, answers: (PauseCheckVerdict | Error)[]) {
    const api = new CheckingApi([finish], answers);
    const paused = vi.fn();
    const runner = new InterviewRunner({
      api,
      audio: new FakeAudio(),
      now: () => Date.now(),
      random: () => 0.5,
      onAnswerPaused: paused,
    });
    const result = runner.run(first);
    return { api, runner, paused, result };
  }

  /** 题目播两秒，候选人接着说三秒。 */
  async function answerOnce(runner: InterviewRunner<number>) {
    await vi.advanceTimersByTimeAsync(0);
    await quiet(runner, 2_000);
    await feed(runner, 3_000, true);
  }

  it("说完了：不等满两秒就上报，上报的时刻和问的时候是同一组", async () => {
    const { api, runner, paused, result } = start(live("q1"), ["done"]);
    await answerOnce(runner);

    await quiet(runner, 800);
    expect(api.checks).toHaveLength(1);
    expect(paused).toHaveBeenCalledTimes(1);
    expect(api.requests).toHaveLength(0);

    await quiet(runner, 600);
    expect(api.requests).toHaveLength(1);
    const answer = api.requests[0] as AdvanceRequest;
    expect(answer.end_reason).toBe("silence");
    expect(api.checks[0]).toEqual({
      seq: answer.seq,
      utterance_key: "q1",
      playback_end_ms: answer.playback_end_ms,
      answer_start_ms: answer.answer_start_ms,
      answer_end_ms: answer.answer_end_ms,
    });
    await expect(result).resolves.toBe("completed");
  });

  it("没说完：过了两秒也不上报，多等一会儿", async () => {
    const { api, runner, result } = start(live("q1"), ["wait"]);
    await answerOnce(runner);

    await quiet(runner, 3_000);
    expect(api.checks).toHaveLength(1);
    expect(api.requests).toHaveLength(0);
    expect(runner.getSnapshot().phase).toBe("listening");

    await quiet(runner, 1_000);
    expect(api.requests).toHaveLength(1);
    await expect(result).resolves.toBe("completed");
  });

  it("判断还没回来候选人就接着说了：这个判断不算数", async () => {
    const { api, runner, result } = start(live("q1"), ["done", "unknown"]);
    await answerOnce(runner);

    await quiet(runner, 600); // 停了一下，这时已经去问了
    expect(api.checks).toHaveLength(1);
    await feed(runner, 2_000, true); // 判断回来时又在说了
    expect(api.requests).toHaveLength(0);

    // 第二次停顿另问一次；服务端说不知道，就按两秒算
    await quiet(runner, 1_800);
    expect(api.checks).toHaveLength(2);
    expect(api.checks[1]?.answer_end_ms).toBeGreaterThan(api.checks[0]?.answer_end_ms as number);
    expect(api.requests).toHaveLength(0);
    await quiet(runner, 600);
    expect(api.requests).toHaveLength(1);
    await expect(result).resolves.toBe("completed");
  });

  it("问不到就按原来的静音时长；一段回答里问的次数有上限", async () => {
    const { api, runner, result } = start(live("q1", 2), [new Error("断网了"), "unknown"]);
    await answerOnce(runner);

    await quiet(runner, 1_000);
    await feed(runner, 1_000, true);
    await quiet(runner, 1_000);
    await feed(runner, 1_000, true);
    expect(api.checks).toHaveLength(2);

    // 第三次停顿不再问了
    await quiet(runner, 1_800);
    expect(api.checks).toHaveLength(2);
    expect(api.requests).toHaveLength(0);
    await quiet(runner, 600);
    expect(api.requests).toHaveLength(1);
    await expect(result).resolves.toBe("completed");
  });

  it("逐题面试不问", async () => {
    const { api, runner, paused, result } = start(say("q1"), ["done"]);
    await answerOnce(runner);
    await quiet(runner, 6_000);
    expect(api.checks).toHaveLength(0);
    expect(paused).not.toHaveBeenCalled();
    expect(api.requests).toHaveLength(1);
    await expect(result).resolves.toBe("completed");
  });
});

describe("候选人插话", () => {
  const listenAfter = { silence_sec: 2, max_sec: 60, min_sec: 1 };

  /** 实时对话里可以插话的一句：出声就停，再持续 400 毫秒算插话，一句里最多停两次。 */
  function interruptible(
    key: string,
    options: { expectsAnswer?: boolean; audioMs?: number } = {},
    answer: AnswerSettings = listenAfter,
  ): NextStep {
    const step = say(key, options);
    if (step.action !== "say") throw new Error("不会发生");
    return {
      ...step,
      answer: step.expects_answer ? answer : null,
      barge_in: { confirm_ms: 400, max_holds: 2, answer },
    };
  }

  it("面试官说到一半候选人接着说：停下来听，说完上报断在哪", async () => {
    const { api, audio, runner, phases } = setup([finish]);
    const result = runner.run(interruptible("q1", { audioMs: 4_000 }));
    await vi.advanceTimersByTimeAsync(0);

    await quiet(runner, 1_000);
    await feed(runner, 300, true);
    // 一听出候选人出声就不播了，这时还不知道是不是插话
    expect(runner.getSnapshot().phase).toBe("asking");
    await feed(runner, 1_700, true);
    expect(runner.getSnapshot()).toMatchObject({ phase: "listening", speaking: true });
    await quiet(runner, 2_600);

    expect(api.requests).toHaveLength(1);
    const request = api.requests[0] as AdvanceRequest;
    // 候选人第 1 秒开口，过 150 毫秒听出来
    expect(request.interrupted_played_ms).toBeGreaterThanOrEqual(1_100);
    expect(request.interrupted_played_ms).toBeLessThanOrEqual(1_250);
    expect(request).toMatchObject({
      utterance_key: "q1",
      playback_start_ms: 0,
      playback_end_ms: request.interrupted_played_ms,
      answer_start_ms: request.interrupted_played_ms,
      end_reason: "silence",
    });
    expect(request.answer_end_ms).toBeGreaterThanOrEqual(3_000);
    expect(request.answer_end_ms).toBeLessThanOrEqual(3_500);
    expect(audio.startedFrom).toEqual([0]); // 没有接着播
    expect(phases).toEqual(["preparing", "asking", "listening", "submitting", "finished"]);
    await expect(result).resolves.toBe("completed");
  });

  it("只是咳嗽一声：从停下的地方往回一点接着播，照常播完", async () => {
    const { api, audio, runner } = setup([finish]);
    void runner.run(interruptible("opening", { expectsAnswer: false, audioMs: 4_000 }));
    await vi.advanceTimersByTimeAsync(0);

    await quiet(runner, 1_000);
    await feed(runner, 300, true);
    await quiet(runner, 300);
    expect(audio.startedFrom).toEqual([0]); // 停着，在等候选人是不是接着说
    await quiet(runner, 4_000);

    expect(audio.startedFrom).toHaveLength(2);
    const resumedFrom = audio.startedFrom[1] as number;
    expect(resumedFrom).toBeGreaterThanOrEqual(850);
    expect(resumedFrom).toBeLessThanOrEqual(1_000);
    expect(api.requests).toHaveLength(1);
    expect(api.requests[0]).toEqual({
      seq: expect.any(Number),
      utterance_key: "opening",
      playback_start_ms: 0,
      playback_end_ms: expect.any(Number),
      answer_start_ms: null,
      answer_end_ms: null,
      end_reason: null,
      replays: 0,
    });
    runner.stop();
  });

  it("不用听回答的一句也能被打断", async () => {
    const { api, runner } = setup([finish]);
    void runner.run(interruptible("opening", { expectsAnswer: false, audioMs: 4_000 }));
    await vi.advanceTimersByTimeAsync(0);

    await quiet(runner, 500);
    await feed(runner, 1_500, true);
    expect(runner.getSnapshot().phase).toBe("listening");
    await quiet(runner, 2_600);

    expect(api.requests).toHaveLength(1);
    const request = api.requests[0] as AdvanceRequest;
    expect(request.end_reason).toBe("silence");
    expect(request.interrupted_played_ms).toBeGreaterThanOrEqual(600);
    expect(request.answer_start_ms).toBe(request.playback_end_ms);
    runner.stop();
  });

  it("一句里最多停两次，之后一口气播完", async () => {
    const { audio, runner } = setup([finish]);
    void runner.run(interruptible("opening", { expectsAnswer: false, audioMs: 8_000 }));
    await vi.advanceTimersByTimeAsync(0);

    for (let cough = 0; cough < 3; cough += 1) {
      await quiet(runner, 1_000);
      await feed(runner, 300, true);
      await quiet(runner, 700);
    }
    expect(audio.startedFrom).toHaveLength(3); // 开头一次，停了两次各接着播一次；第三声没停
    runner.stop();
  });

  it("连着好几次都不是插话：这场面试不再为插话停", async () => {
    const steps = [1, 2, 3].map((n) =>
      interruptible(`s${n}`, { expectsAnswer: false, audioMs: n === 3 ? 20_000 : 5_000 }),
    );
    const { api, audio, runner } = setup([steps[1] as NextStep, steps[2] as NextStep, finish]);
    void runner.run(steps[0] as NextStep);
    await vi.advanceTimersByTimeAsync(0);

    // 前两句各有两声咳嗽，一共白停了四次
    for (let n = 0; n < 2; n += 1) {
      for (let cough = 0; cough < 2; cough += 1) {
        await quiet(runner, 800);
        await feed(runner, 300, true);
        await quiet(runner, 700);
      }
      await quiet(runner, 5_000);
      expect(api.requests).toHaveLength(n + 1);
    }
    const resumes = () => audio.startedFrom.filter((fromMs) => fromMs > 0).length;
    expect(resumes()).toBe(4);

    // 第三句再有声音也不停了；候选人真的长篇说话也不停，等这一句播完
    await quiet(runner, 800);
    await feed(runner, 1_500, true);
    expect(resumes()).toBe(4);
    expect(runner.getSnapshot()).toMatchObject({
      phase: "asking",
      utterance: { key: "s3" },
    });
    runner.stop();
  });

  it("面试官要开口时候选人正说着话：先不播，接着说就直接听", async () => {
    const { api, audio, runner } = setup([finish]);
    await quiet(runner, 1_000);
    await feed(runner, 600, true); // 上一句答完之后候选人又说了起来
    void runner.run(interruptible("q1"));
    await feed(runner, 1_400, true);
    expect(runner.getSnapshot()).toMatchObject({ phase: "listening", speaking: true });
    await quiet(runner, 2_600);

    expect(audio.startedFrom).toEqual([]); // 一个字都没播
    expect(api.requests).toHaveLength(1);
    expect(api.requests[0]).toMatchObject({ interrupted_played_ms: 0, end_reason: "silence" });
    runner.stop();
  });

  it("插话之后停顿时先问服务端，也带上断在哪", async () => {
    const checks: PauseCheckRequest[] = [];
    const api = Object.assign(new FakeApi([finish]), {
      checkPause: async (request: PauseCheckRequest): Promise<PauseCheckVerdict> => {
        checks.push(request);
        return "done";
      },
    });
    const runner = new InterviewRunner({ api, audio: new FakeAudio(), now: () => Date.now() });
    const check = { after_ms: 500, done_silence_ms: 1_000, wait_silence_ms: 3_500, max_checks: 8 };
    void runner.run(
      interruptible("opening", { expectsAnswer: false }, { ...listenAfter, pause_check: check }),
    );
    await vi.advanceTimersByTimeAsync(0);

    await quiet(runner, 500);
    await feed(runner, 1_500, true);
    await quiet(runner, 1_500);

    expect(checks).toHaveLength(1);
    expect(api.requests).toHaveLength(1); // 服务端说说完了，静音满一秒就上报
    const request = api.requests[0] as AdvanceRequest;
    expect(request.interrupted_played_ms).toBeGreaterThan(0);
    expect(checks[0]).toEqual({
      seq: request.seq,
      utterance_key: "opening",
      playback_end_ms: request.playback_end_ms,
      answer_start_ms: request.answer_start_ms,
      answer_end_ms: request.answer_end_ms,
      interrupted_played_ms: request.interrupted_played_ms,
    });
    runner.stop();
  });

  it("麦克风被打断、没有音量可判断时，不会一直停着", async () => {
    const { api, audio, runner } = setup([finish]);
    void runner.run(interruptible("opening", { expectsAnswer: false, audioMs: 3_000 }));
    await vi.advanceTimersByTimeAsync(0);

    await quiet(runner, 500);
    await feed(runner, 200, true); // 停下来了，然后麦克风没了动静
    await vi.advanceTimersByTimeAsync(6_000);
    expect(audio.startedFrom).toHaveLength(2);
    expect(api.requests).toHaveLength(1);
    runner.stop();
  });
});

describe("作答设置", () => {
  it("停顿时先问的设置：问得比静音时长还晚就不问，两个时长夹在静音时长两边", () => {
    const check = { after_ms: 500, done_silence_ms: 1_000, wait_silence_ms: 3_500, max_checks: 8 };
    const answer = { silence_sec: 2, max_sec: 20, min_sec: 1 };
    expect(turnConfigFor({ ...answer, pause_check: check }).pauseCheck).toEqual({
      afterMs: 500,
      doneSilenceMs: 1_000,
      waitSilenceMs: 3_500,
      maxChecks: 8,
    });
    expect(
      turnConfigFor({
        ...answer,
        pause_check: { ...check, done_silence_ms: 5_000, wait_silence_ms: 1_000 },
      }).pauseCheck,
    ).toMatchObject({ doneSilenceMs: 2_000, waitSilenceMs: 2_000 });
    expect(
      turnConfigFor({ ...answer, pause_check: { ...check, after_ms: 2_000 } }).pauseCheck,
    ).toBeUndefined();
    expect(turnConfigFor({ ...answer, pause_check: null }).pauseCheck).toBeUndefined();
    expect(turnConfigFor(answer).pauseCheck).toBeUndefined();
  });

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
