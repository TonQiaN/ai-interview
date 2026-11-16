/**
 * 一道题从提问到答完的状态机（逐题模式）。
 *
 * 纯逻辑：所有时间都由事件带进来（毫秒，单调时钟），不读系统时间、不碰浏览器接口。
 * 谁来驱动它：interview-runner 把「题目播完了」「VAD 说开口了」「用户点了按钮」「又过了一会儿」
 * 这些事翻译成事件喂进来，再根据返回的状态更新画面。
 *
 * 流程：
 *   asking（播放题目）→ listening（听作答）→ ended（这道题结束，带原因）
 * 实时对话里候选人可以插话：asking 时听出候选人开口并且接着说，直接进 listening。
 * 题目语音放不出来时走 reading（只显示文字，候选人点「开始回答」后进入 listening）。
 */

export interface TurnConfig {
  /** 开口之后，连续静音多久算答完（毫秒）。 */
  silenceMs: number;
  /** 静音判定的最后多久显示倒计时（毫秒）。 */
  countdownMs: number;
  /** 题目播完后一直没开口，多久提示一次「请开始回答」（毫秒）。 */
  noSpeechPromptMs: number;
  /** 题目播完后一直没开口，多久直接进入下一题（毫秒）。 */
  noSpeechTimeoutMs: number;
  /** 开始听作答之后，至少过这么久才允许因为静音结束（毫秒）。防止咳嗽一声就被判成答完。 */
  minAnswerMs: number;
  /** 一道题最长作答时间（毫秒），从题目播完算起。 */
  maxAnswerMs: number;
  /** 离最长作答时间还剩多久时提醒（毫秒）。 */
  maxAnswerWarningMs: number;
  /** 可以重听题目几次。 */
  maxReplays: number;
  /** 候选人停顿时先问服务端「说完了吗」（实时对话）。没有就只按 silenceMs 算。 */
  pauseCheck?: PauseCheckConfig;
}

export interface PauseCheckConfig {
  /** 声音停了多久去问（毫秒）。 */
  afterMs: number;
  /** 服务端觉得说完了：这次停顿静音满这么久就算答完（毫秒），比 silenceMs 短。 */
  doneSilenceMs: number;
  /** 服务端觉得没说完：这次停顿多等一会儿，静音满这么久才算答完（毫秒），比 silenceMs 长。 */
  waitSilenceMs: number;
  /** 一段回答里最多问几次。 */
  maxChecks: number;
}

/** 服务端对一次停顿的判断：说完了，还是没说完。 */
export type PauseVerdict = "done" | "wait";

export const DEFAULT_TURN_CONFIG: TurnConfig = {
  silenceMs: 5_000,
  countdownMs: 3_000,
  noSpeechPromptMs: 12_000,
  noSpeechTimeoutMs: 30_000,
  minAnswerMs: 3_000,
  maxAnswerMs: 180_000,
  maxAnswerWarningMs: 15_000,
  maxReplays: 2,
};

/** 这道题为什么结束。 */
export type TurnEndReason =
  /** 开口后静音达到设定时长。 */
  | "silence"
  /** 候选人点了「我答完了」。 */
  | "manual"
  /** 一直没有开口。 */
  | "no_speech"
  /** 到了最长作答时间。 */
  | "max_duration";

export type TurnState =
  | {
      phase: "asking";
      replaysUsed: number;
      /** 本次播放开始的时刻。 */
      askStartedAt: number;
    }
  | {
      phase: "reading";
      replaysUsed: number;
      askStartedAt: number;
    }
  | {
      phase: "listening";
      replaysUsed: number;
      /** 开始听作答的时刻（题目播完，或候选人点了「开始回答」）。 */
      listenStartedAt: number;
      /** 第一次开口的时刻；还没开口是 null。 */
      firstSpeechAt: number | null;
      /** 此刻是否在说话。 */
      speaking: boolean;
      /** 静音从什么时候开始算；在说话或还没开过口时是 null。 */
      silenceSince: number | null;
      /** 最后一次听出「停止说话」的时刻；在说话或还没开过口时是 null。回答的结束时刻用它。 */
      speechEndedAt: number | null;
      /**
       * 服务端对某次停顿的判断，`silenceSince` 记的是哪一次停顿。候选人接着说了，或者点了
       * 「继续回答」，停顿就不是那一次了，这个判断自然作废。
       */
      pauseVerdict: { verdict: PauseVerdict; silenceSince: number } | null;
      /** 是否已经提示过「请开始回答」。 */
      promptShown: boolean;
      /** 暂停开始的时刻（设备被打断等）；没有暂停是 null。 */
      pausedAt: number | null;
    }
  | {
      phase: "ended";
      reason: TurnEndReason;
      replaysUsed: number;
      listenStartedAt: number;
      firstSpeechAt: number | null;
      /** 最后一次停止说话的时刻；没开过口是 null。 */
      lastSpeechEndAt: number | null;
      endedAt: number;
    };

export type TurnEvent =
  /** 题目语音播完。 */
  | { type: "question_ended"; at: number }
  /** 题目语音放不出来，改为让候选人自己读。 */
  | { type: "question_failed"; at: number }
  /**
   * 面试官还在说，候选人插话了（实时对话）：不再往下播，直接听候选人说。
   * `speechAt` 是听出候选人开口的时刻，这时候选人还在说。
   */
  | { type: "barge_in"; at: number; speechAt: number }
  /** reading 状态下候选人点了「开始回答」。 */
  | { type: "user_ready"; at: number }
  | { type: "speech_start"; at: number }
  /**
   * 听出候选人停止说话。`quietMs` 是这时已经安静了多久：VAD 要等声音停一小会儿才敢下结论，
   * 静音从声音真正停下的那一刻算起，不然每次都多等这一小会儿。
   */
  | { type: "speech_end"; at: number; quietMs?: number }
  /** 候选人点了「我答完了」。 */
  | { type: "user_done"; at: number }
  /**
   * 服务端判断完了一次停顿。`silenceSince` 是问的时候那次停顿开始的时刻：
   * 结果回来时候选人要是已经接着说了，就不是同一次停顿，不算数。
   */
  | { type: "pause_verdict"; at: number; verdict: PauseVerdict; silenceSince: number }
  /** 倒计时期间候选人点了「继续回答」。 */
  | { type: "user_continue"; at: number }
  /** 候选人点了「重听题目」。 */
  | { type: "user_replay"; at: number }
  /** 摄像头或麦克风被打断，暂停计时。 */
  | { type: "pause"; at: number }
  | { type: "resume"; at: number }
  /** 时间流逝。驱动方按固定间隔发。 */
  | { type: "tick"; at: number };

export function startTurn(at: number): TurnState {
  return { phase: "asking", replaysUsed: 0, askStartedAt: at };
}

function startListening(at: number, replaysUsed: number): Listening {
  return {
    phase: "listening",
    replaysUsed,
    listenStartedAt: at,
    firstSpeechAt: null,
    speaking: false,
    silenceSince: null,
    speechEndedAt: null,
    pauseVerdict: null,
    promptShown: false,
    pausedAt: null,
  };
}

type Listening = Extract<TurnState, { phase: "listening" }>;

function end(state: Listening, reason: TurnEndReason, at: number): TurnState {
  return {
    phase: "ended",
    reason,
    replaysUsed: state.replaysUsed,
    listenStartedAt: state.listenStartedAt,
    firstSpeechAt: state.firstSpeechAt,
    lastSpeechEndAt: state.speaking ? at : state.speechEndedAt,
    endedAt: at,
  };
}

/**
 * 这次停顿静音多久算答完。一般是 silenceMs；服务端判断过这次停顿的话，
 * 说完了就短一些，没说完就长一些。
 */
function silenceLimitMs(state: Listening, config: TurnConfig): number {
  const check = config.pauseCheck;
  const judged = state.pauseVerdict;
  if (!check || !judged || judged.silenceSince !== state.silenceSince) return config.silenceMs;
  return judged.verdict === "done"
    ? Math.min(config.silenceMs, check.doneSilenceMs)
    : Math.max(config.silenceMs, check.waitSilenceMs);
}

/** 检查各种超时。只在没有暂停时调用。 */
function checkTimeouts(state: Listening, at: number, config: TurnConfig): TurnState {
  if (at - state.listenStartedAt >= config.maxAnswerMs) {
    return end(state, "max_duration", at);
  }
  if (state.firstSpeechAt === null) {
    const waited = at - state.listenStartedAt;
    if (waited >= config.noSpeechTimeoutMs) return end(state, "no_speech", at);
    if (!state.promptShown && waited >= config.noSpeechPromptMs) {
      return { ...state, promptShown: true };
    }
    return state;
  }
  if (
    state.silenceSince !== null &&
    at - state.silenceSince >= silenceLimitMs(state, config) &&
    at - state.listenStartedAt >= config.minAnswerMs
  ) {
    return end(state, "silence", at);
  }
  return state;
}

export function reduceTurn(
  state: TurnState,
  event: TurnEvent,
  config: TurnConfig = DEFAULT_TURN_CONFIG,
): TurnState {
  switch (state.phase) {
    case "asking":
      if (event.type === "question_ended") return startListening(event.at, state.replaysUsed);
      if (event.type === "question_failed") return { ...state, phase: "reading" };
      if (event.type === "barge_in") {
        const speechAt = Math.min(event.speechAt, event.at);
        return {
          ...startListening(speechAt, state.replaysUsed),
          firstSpeechAt: speechAt,
          speaking: true,
        };
      }
      return state;

    case "reading":
      if (event.type === "user_ready") return startListening(event.at, state.replaysUsed);
      return state;

    case "listening": {
      if (state.pausedAt !== null) {
        if (event.type !== "resume") return state;
        // 暂停期间不计时：把所有时间点整体往后挪。暂停前如果在说话，恢复后从静音算起。
        const shift = event.at - state.pausedAt;
        return {
          ...state,
          pausedAt: null,
          listenStartedAt: state.listenStartedAt + shift,
          firstSpeechAt: state.firstSpeechAt === null ? null : state.firstSpeechAt + shift,
          speaking: false,
          silenceSince: state.firstSpeechAt === null ? null : event.at,
          speechEndedAt: state.firstSpeechAt === null ? null : event.at,
        };
      }

      switch (event.type) {
        case "pause":
          return { ...state, pausedAt: event.at };

        case "speech_start": {
          const next = checkTimeouts(state, event.at, config);
          if (next.phase !== "listening") return next;
          return {
            ...next,
            speaking: true,
            silenceSince: null,
            speechEndedAt: null,
            firstSpeechAt: next.firstSpeechAt ?? event.at,
          };
        }

        case "speech_end": {
          if (!state.speaking) return state;
          // 声音是 quietMs 之前停的。时钟再怎么抖，也不会早于候选人开口
          const quietSince = Math.max(
            event.at - Math.max(0, event.quietMs ?? 0),
            state.firstSpeechAt ?? event.at,
          );
          return checkTimeouts(
            { ...state, speaking: false, silenceSince: quietSince, speechEndedAt: event.at },
            event.at,
            config,
          );
        }

        case "pause_verdict":
          if (state.speaking || state.silenceSince !== event.silenceSince) return state;
          return checkTimeouts(
            {
              ...state,
              pauseVerdict: { verdict: event.verdict, silenceSince: event.silenceSince },
            },
            event.at,
            config,
          );

        case "user_continue":
          // 只有在静音计时期间才有意义：重新开始计静音。
          if (state.silenceSince === null) return state;
          return { ...state, silenceSince: event.at };

        case "user_done":
          return end(state, "manual", event.at);

        case "user_replay":
          // 开口之后就不能重听了，免得把答到一半的回答打断。
          if (state.firstSpeechAt !== null || state.replaysUsed >= config.maxReplays) return state;
          return { phase: "asking", replaysUsed: state.replaysUsed + 1, askStartedAt: event.at };

        case "tick":
          return checkTimeouts(state, event.at, config);

        default:
          return state;
      }
    }

    case "ended":
      return state;
  }
}

/** 画面需要的派生信息。 */
export interface TurnView {
  /** 是否显示静音倒计时。 */
  countdownVisible: boolean;
  /** 离静音判定还剩多少毫秒；没在计静音时是 null。 */
  silenceRemainingMs: number | null;
  /** 离最长作答时间还剩多少毫秒；不在 listening 时是 null。 */
  answerRemainingMs: number | null;
  /** 是否提醒「时间快到了」。 */
  maxWarningVisible: boolean;
  /** 是否提示「请开始回答」。 */
  noSpeechPromptVisible: boolean;
  /** 还能不能重听题目。 */
  canReplay: boolean;
}

export function viewTurn(
  state: TurnState,
  now: number,
  config: TurnConfig = DEFAULT_TURN_CONFIG,
): TurnView {
  if (state.phase !== "listening") {
    return {
      countdownVisible: false,
      silenceRemainingMs: null,
      answerRemainingMs: null,
      maxWarningVisible: false,
      noSpeechPromptVisible: false,
      canReplay: false,
    };
  }
  const at = state.pausedAt ?? now;
  const silenceRemainingMs =
    state.silenceSince === null
      ? null
      : Math.max(
          0,
          silenceLimitMs(state, config) - (at - state.silenceSince),
          config.minAnswerMs - (at - state.listenStartedAt),
        );
  const answerRemainingMs = Math.max(0, config.maxAnswerMs - (at - state.listenStartedAt));
  return {
    countdownVisible: silenceRemainingMs !== null && silenceRemainingMs <= config.countdownMs,
    silenceRemainingMs,
    answerRemainingMs,
    maxWarningVisible: answerRemainingMs <= config.maxAnswerWarningMs,
    noSpeechPromptVisible: state.promptShown && state.firstSpeechAt === null,
    canReplay: state.firstSpeechAt === null && state.replaysUsed < config.maxReplays,
  };
}
