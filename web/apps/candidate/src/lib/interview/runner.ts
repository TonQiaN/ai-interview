/**
 * 面试的编排：把「播一句 → 听回答 → 上报 → 拿下一句」这个循环跑起来。逐题模式和实时对话都用它。
 *
 * 实时对话里服务端允许的话（「下一步」带 `barge_in`），面试官说话时候选人可以插话：
 * 一听到候选人出声就先停下来，接着说就改成听候选人说，只是咳嗽一声就从停下的地方接着播。
 *
 * 它不直接碰浏览器接口：音频播放、接口调用、时钟都从外面传进来，所以能在 Node 里用假的实现测。
 * 画面通过 subscribe 拿快照，按钮点击调 done / continueAnswer / replay / ready。
 *
 * 断网时 advance 会一直重试（录制不受影响，分片先存在本地），候选人看到的是「正在重新连接」。
 * 只有链接失效、面试被取消这类重试没用的错误才会让 run 失败。
 */
import { RoomApiError } from "../api/room-api";
import type {
  AdvanceRequest,
  AnswerEndReason,
  AnswerSettings,
  NextStep,
  PauseCheckRequest,
  PauseCheckVerdict,
  Progress,
  SayStep,
  Utterance,
} from "../api/room-types";
import type { LevelFrame, Playback } from "../media/audio-graph";
import {
  DEFAULT_TURN_CONFIG,
  reduceTurn,
  startTurn,
  type TurnConfig,
  type TurnEndReason,
  type TurnEvent,
  type TurnState,
  type TurnView,
  viewTurn,
} from "../turn/turn-machine";
import { retryDelayMs } from "../upload/uploader";
import { EnergyVad } from "../vad/energy-vad";

/** 播放面试官语音的东西。浏览器里是 AudioGraph。 */
export interface AudioPlayer<B> {
  decode(data: ArrayBuffer): Promise<B>;
  /** `fromMs`：从这段音频的哪里开始播，不给就是从头。 */
  play(buffer: B, fromMs?: number): Playback;
}

export interface RunnerApi {
  advance(request: AdvanceRequest): Promise<NextStep>;
  /** 候选人停顿时问服务端「说完了吗」。没有这个方法就不问。 */
  checkPause?(request: PauseCheckRequest): Promise<PauseCheckVerdict>;
  fetchAudio(audioUrl: string): Promise<ArrayBuffer>;
}

export interface RunnerOptions<B> {
  api: RunnerApi;
  audio: AudioPlayer<B>;
  /** 面试时间轴上的当前时刻（毫秒）。 */
  now: () => number;
  vad?: EnergyVad;
  /** 多久检查一次超时、刷新一次画面（毫秒）。 */
  tickMs?: number;
  /** 语音多久取不下来就改成只显示文字（毫秒）。 */
  audioTimeoutMs?: number;
  /** 上报失败后的重试间隔：从 base 开始翻倍，最多 max（毫秒）。 */
  retryBaseMs?: number;
  retryMaxMs?: number;
  random?: () => number;
  /** 每次状态机发生有意义的变化时调用，用来记过程事件。 */
  onTurnEvent?: (event: TurnEvent, state: TurnState) => void;
  /** 候选人答完一题、还没上报之前调用。 */
  onAnswerEnded?: () => void;
  /** 候选人停顿了、要去问服务端说完没有之前调用。 */
  onAnswerPaused?: () => void;
}

export type RunnerPhase =
  /** 正在取这一句的语音。 */
  | "preparing"
  /** 面试官在说话。 */
  | "asking"
  /** 语音放不出来，只显示文字。需要回答时等候选人点「开始回答」。 */
  | "reading"
  /** 在听候选人回答。 */
  | "listening"
  /** 正在上报并取下一句。 */
  | "submitting"
  | "finished"
  /** 出了重试也没用的错（链接失效等）。 */
  | "failed";

export interface RunnerSnapshot {
  phase: RunnerPhase;
  utterance: Utterance | null;
  expectsAnswer: boolean;
  progress: Progress | null;
  /** 听作答时的倒计时等信息；其他阶段是 null。 */
  turn: TurnView | null;
  /** 候选人此刻是否在说话。 */
  speaking: boolean;
  /** 麦克风音量，0 到 1，给音量条用。 */
  level: number;
  /** 这一句有没有语音可以重听。 */
  canReplay: boolean;
  paused: boolean;
  /** 上报连续失败了几次；大于 0 说明正在重新连接。 */
  submitFailures: number;
  /** 面试结束的原因；没结束是 null。 */
  finishReason: string | null;
  error: RoomApiError | null;
}

const END_REASON: Record<TurnEndReason, AnswerEndReason> = {
  silence: "silence",
  manual: "button",
  no_speech: "no_speech",
  max_duration: "timeout",
};

/** 把服务端给的作答设置换成状态机的配置。 */
export function turnConfigFor(
  answer: AnswerSettings | null | undefined,
  base: TurnConfig = DEFAULT_TURN_CONFIG,
): TurnConfig {
  if (!answer) return base;
  const silenceMs = Math.max(1_000, Math.round(answer.silence_sec * 1000));
  const maxAnswerMs = Math.max(10_000, Math.round(answer.max_sec * 1000));
  const check = answer.pause_check;
  const config: TurnConfig = { ...base };
  delete config.pauseCheck;
  // 问得比静音时长还晚就没有意义了
  if (check && check.max_checks > 0 && check.after_ms < silenceMs) {
    config.pauseCheck = {
      afterMs: check.after_ms,
      doneSilenceMs: Math.min(silenceMs, Math.max(check.after_ms, check.done_silence_ms)),
      waitSilenceMs: Math.max(silenceMs, check.wait_silence_ms),
      maxChecks: check.max_checks,
    };
  }
  return {
    ...config,
    silenceMs,
    // 静音时间设得很短时，倒计时从一开始就显示
    countdownMs: Math.min(base.countdownMs, silenceMs),
    minAnswerMs: Math.max(0, Math.round(answer.min_sec * 1000)),
    maxAnswerMs,
    maxAnswerWarningMs: Math.min(base.maxAnswerWarningMs, Math.round(maxAnswerMs / 4)),
    // 一直不开口的等待时间不能比最长作答时间还长
    noSpeechTimeoutMs: Math.min(base.noSpeechTimeoutMs, maxAnswerMs),
  };
}

/** 没有语音、也不需要回答的一句话，在屏幕上停多久（毫秒）：按每秒五个字估，至少三秒。 */
export function readingTimeMs(text: string): number {
  return Math.min(20_000, Math.max(3_000, text.length * 200));
}

/** 把分贝换成 0 到 1 的音量条读数：-60 分贝以下是 0，-10 分贝以上是 1。 */
export function levelFromDb(db: number): number {
  return Math.min(1, Math.max(0, (db + 60) / 50));
}

/** 疑似插话之后接着播，往回退这么多（毫秒），免得断掉的那个字听不全。 */
const RESUME_REWIND_MS = 250;
/** 确认是插话的那一刻，候选人的声音最多可以已经低下去这么久（毫秒）：字和字之间的空隙不算停。 */
const BARGE_IN_GAP_MS = 150;
/** 停下来之后麦克风一直没有动静可判断（设备被打断了），过了确认时间再等这么久就接着播（毫秒）。 */
const HOLD_GIVE_UP_MS = 2_000;
/**
 * 连着这么多次停下来都不是插话（多半是扬声器的声音漏进了麦克风，或者环境太吵），
 * 这场面试就不再为插话停了，免得面试官说话一卡一卡的。
 */
export const MAX_FALSE_HOLDS = 4;

/** 面试官的一句话被候选人插话打断的地方。 */
interface Cut {
  /** 听出候选人开口、停下播放的时刻。 */
  at: number;
  /** 停下时这一句播了多少毫秒。 */
  playedMs: number;
}

class Aborted extends Error {}

export class InterviewRunner<B> {
  private readonly api: RunnerApi;
  private readonly audio: AudioPlayer<B>;
  private readonly now: () => number;
  private readonly vad: EnergyVad;
  private readonly tickMs: number;
  private readonly audioTimeoutMs: number;
  private readonly retryBaseMs: number;
  private readonly retryMaxMs: number;
  private readonly random: () => number;
  private readonly onTurnEvent: ((event: TurnEvent, state: TurnState) => void) | undefined;
  private readonly onAnswerEnded: (() => void) | undefined;
  private readonly onAnswerPaused: (() => void) | undefined;

  private readonly listeners = new Set<(snapshot: RunnerSnapshot) => void>();
  private readonly audioCache = new Map<string, Promise<B | null>>();

  private phase: RunnerPhase = "preparing";
  private step: SayStep | null = null;
  private turn: TurnState | null = null;
  private config: TurnConfig = DEFAULT_TURN_CONFIG;
  private hasAudio = false;
  private submitFailures = 0;
  private finishReason: string | null = null;
  private error: RoomApiError | null = null;
  private snapshot: RunnerSnapshot;

  /** 这一句播完的时刻。 */
  private playbackEnd = 0;
  /** 这一句的回答里已经问过几次「说完了吗」。 */
  private pauseChecks = 0;
  /** 最近问的是哪一次停顿（那次停顿开始的时刻）。同一次停顿只问一次。 */
  private checkedSilenceSince: number | null = null;

  /** 这一句是被候选人插话打断的：断的时候播了多少毫秒。没被打断是 null。 */
  private cutPlayedMs: number | null = null;
  /** 面试官正在说、候选人可以插话时：听出候选人开口就调它，把播放停下来。 */
  private onBargeSpeech: (() => void) | null = null;
  /** 为疑似插话停了下来，正在等候选人是不是接着说。 */
  private hold: { at: number; confirmMs: number; settle: (confirmed: boolean) => void } | null =
    null;
  /** 连着几次停下来都不是插话。 */
  private falseHolds = 0;

  private playback: Playback | null = null;
  private ticker: ReturnType<typeof setInterval> | null = null;
  private aborted = false;
  /** 等状态机走到某个阶段的人。 */
  private turnWaiter: { until: (state: TurnState) => boolean; resolve: () => void } | null = null;
  private readonly abortWaiters = new Set<() => void>();

  constructor(options: RunnerOptions<B>) {
    this.api = options.api;
    this.audio = options.audio;
    this.now = options.now;
    this.vad = options.vad ?? new EnergyVad();
    this.tickMs = options.tickMs ?? 100;
    this.audioTimeoutMs = options.audioTimeoutMs ?? 8_000;
    this.retryBaseMs = options.retryBaseMs ?? 1_000;
    this.retryMaxMs = options.retryMaxMs ?? 10_000;
    this.random = options.random ?? Math.random;
    this.onTurnEvent = options.onTurnEvent;
    this.onAnswerEnded = options.onAnswerEnded;
    this.onAnswerPaused = options.onAnswerPaused;
    this.snapshot = this.buildSnapshot();
  }

  getSnapshot = (): RunnerSnapshot => this.snapshot;

  subscribe = (listener: (snapshot: RunnerSnapshot) => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /**
   * 从第一步开始把整场面试跑完。正常结束时返回结束原因；
   * 链接失效这类错误会抛出 RoomApiError；被 stop 打断时返回 null。
   */
  async run(first: NextStep): Promise<string | null> {
    let step = first;
    try {
      while (step.action === "say") {
        const request = await this.runUtterance(step);
        step = await this.submit(request);
      }
      this.finishReason = step.reason;
      this.setPhase("finished");
      return step.reason;
    } catch (error) {
      if (error instanceof Aborted) return null;
      this.error =
        error instanceof RoomApiError ? error : new RoomApiError("rejected", String(error));
      this.setPhase("failed");
      throw this.error;
    } finally {
      this.stopTicking();
    }
  }

  /** 页面要离开了：停掉播放和计时，不再上报。 */
  stop(): void {
    if (this.aborted) return;
    this.aborted = true;
    this.playback?.stop();
    this.stopTicking();
    this.turnWaiter?.resolve();
    for (const wake of this.abortWaiters) wake();
  }

  // ── 候选人的操作 ──

  /** 「我答完了」。 */
  done(): void {
    this.dispatch({ type: "user_done", at: this.now() });
  }

  /** 倒计时期间点「继续回答」。 */
  continueAnswer(): void {
    this.dispatch({ type: "user_continue", at: this.now() });
  }

  /** 「重听题目」。 */
  replay(): void {
    if (!this.hasAudio) return;
    this.dispatch({ type: "user_replay", at: this.now() });
  }

  /** 只显示文字时点「开始回答」。 */
  ready(): void {
    this.dispatch({ type: "user_ready", at: this.now() });
  }

  /** 麦克风或摄像头被打断：暂停计时。 */
  pause(): void {
    this.dispatch({ type: "pause", at: this.now() });
  }

  resume(): void {
    this.vad.reset();
    this.dispatch({ type: "resume", at: this.now() });
  }

  /** 麦克风的音量帧。AudioGraph 的 onLevel 直接接到这里。 */
  handleLevel = (frame: LevelFrame): void => {
    const event = this.vad.process(frame.rms, frame.frameMs);
    if (this.hold) {
      this.judgeHold(event);
      return;
    }
    if (!event) return;
    if (this.onBargeSpeech) {
      if (event.type === "speech_start") this.onBargeSpeech();
      return;
    }
    if (this.turn?.phase !== "listening" || this.turn.pausedAt !== null) return;
    this.dispatch({ ...event, at: this.now() });
  };

  // ── 内部 ──

  private async runUtterance(step: SayStep): Promise<AdvanceRequest> {
    this.step = step;
    this.config = turnConfigFor(step.answer);
    this.turn = null;
    this.hasAudio = false;
    this.setPhase("preparing");
    for (const url of step.preload_audio_urls) void this.loadAudio(url);

    const buffer = step.utterance.audio_url ? await this.loadAudio(step.utterance.audio_url) : null;
    this.checkAborted();
    this.hasAudio = buffer !== null;

    let playbackStart: number | null = null;
    this.cutPlayedMs = null;
    this.playbackEnd = 0;
    this.pauseChecks = 0;
    this.checkedSilenceSince = null;
    this.turn = startTurn(this.now());

    for (;;) {
      if (buffer) {
        playbackStart ??= this.now();
        this.setPhase("asking");
        const cut = await this.speak(buffer, step);
        if (cut) {
          // 候选人插话了：这一句到此为止，接着听候选人说（哪怕这一句本来不用听回答）
          this.cutPlayedMs = cut.playedMs;
          this.playbackEnd = cut.at;
          this.config = turnConfigFor(step.barge_in?.answer);
          this.dispatch({ type: "barge_in", at: this.now(), speechAt: cut.at });
        } else {
          this.playbackEnd = this.now();
          if (step.expects_answer) this.dispatch({ type: "question_ended", at: this.playbackEnd });
        }
      } else {
        playbackStart ??= this.now();
        this.dispatch({ type: "question_failed", at: playbackStart });
        this.setPhase("reading");
        if (step.expects_answer) {
          await this.waitForTurn((state) => state.phase === "listening");
        } else {
          await this.sleep(readingTimeMs(step.utterance.text));
        }
        this.checkAborted();
        this.playbackEnd = this.now();
      }

      if (!step.expects_answer && this.cutPlayedMs === null) {
        return {
          seq: step.seq,
          utterance_key: step.utterance.key,
          playback_start_ms: playbackStart,
          playback_end_ms: this.playbackEnd,
          answer_start_ms: null,
          answer_end_ms: null,
          end_reason: null,
          replays: 0,
        };
      }

      // 面试官的声音刚停，之前的音量状态不能带进来。插话的不清：候选人正说着
      if (this.cutPlayedMs === null) this.vad.reset();
      this.setPhase("listening");
      this.startTicking();
      await this.waitForTurn((state) => state.phase === "ended" || state.phase === "asking");
      this.stopTicking();
      this.checkAborted();

      const turn = this.turn as TurnState;
      if (turn.phase !== "ended") continue; // 重听题目

      this.onAnswerEnded?.();
      return {
        seq: step.seq,
        utterance_key: step.utterance.key,
        playback_start_ms: playbackStart,
        playback_end_ms: this.playbackEnd,
        answer_start_ms: turn.firstSpeechAt,
        answer_end_ms: turn.lastSpeechEndAt,
        end_reason: END_REASON[turn.reason],
        replays: turn.replaysUsed,
        ...(this.cutPlayedMs === null ? {} : { interrupted_played_ms: this.cutPlayedMs }),
      };
    }
  }

  /**
   * 播面试官的一句话。播完了返回 null；候选人插话打断了，返回断在哪。
   *
   * 服务端允许插话的句子（`barge_in`），一听出候选人出声就先停下来；声音接着持续够久才算插话，
   * 不然（咳嗽、嗯一声、扬声器漏进麦克风的声音）从停下的地方往回退一点接着播。
   */
  private async speak(buffer: B, step: SayStep): Promise<Cut | null> {
    const bargeIn = step.barge_in ?? null;
    let fromMs = 0;
    let holds = 0;
    for (;;) {
      const armed =
        bargeIn !== null && holds < bargeIn.max_holds && this.falseHolds < MAX_FALSE_HOLDS;
      // 要开口的时候候选人正说着话：先不播，看是不是接着说
      const held =
        armed && this.vad.snapshot().speaking
          ? { at: this.now(), playedMs: fromMs }
          : await this.playUntilSpeech(buffer, fromMs, armed);
      this.checkAborted();
      if (held === null || bargeIn === null) return null;

      holds += 1;
      const confirmed = await this.waitForHold(held.at, bargeIn.confirm_ms);
      this.checkAborted();
      if (confirmed) {
        this.falseHolds = 0;
        return held;
      }
      this.falseHolds += 1;
      fromMs = Math.max(0, held.playedMs - RESUME_REWIND_MS);
    }
  }

  /** 从 `fromMs` 开始播。播完返回 null；`armed` 时听出候选人出声就停下，返回停在哪。 */
  private async playUntilSpeech(buffer: B, fromMs: number, armed: boolean): Promise<Cut | null> {
    const playback = this.audio.play(buffer, fromMs);
    this.playback = playback;
    let held: Cut | null = null;
    if (armed) {
      this.onBargeSpeech = () => {
        this.onBargeSpeech = null;
        held = { at: this.now(), playedMs: playback.positionMs() };
        playback.stop();
      };
    }
    await playback.done;
    this.onBargeSpeech = null;
    this.playback = null;
    return held;
  }

  /** 为疑似插话停下来之后，等结果：候选人接着说了（真），还是没声了（假）。 */
  private waitForHold(at: number, confirmMs: number): Promise<boolean> {
    return new Promise((resolve) => {
      const hold = {
        at,
        confirmMs,
        settle: (confirmed: boolean) => {
          if (this.hold !== hold) return;
          this.hold = null;
          resolve(confirmed);
        },
      };
      this.hold = hold;
      void this.sleep(confirmMs + HOLD_GIVE_UP_MS).then(() => {
        if (this.hold !== hold) return;
        // 这么久都没有音量帧来：之前「在说话」的状态已经不作数了
        this.vad.reset();
        hold.settle(false);
      });
    });
  }

  /** 停下来等的时候，每来一帧音量看一次。 */
  private judgeHold(event: ReturnType<EnergyVad["process"]>): void {
    const hold = this.hold;
    if (!hold) return;
    const vad = this.vad.snapshot();
    if (event?.type === "speech_end" || !vad.speaking) {
      hold.settle(false);
    } else if (this.now() - hold.at >= hold.confirmMs && vad.quietMs <= BARGE_IN_GAP_MS) {
      hold.settle(true);
    }
  }

  private async submit(request: AdvanceRequest): Promise<NextStep> {
    this.submitFailures = 0;
    this.setPhase("submitting");
    for (;;) {
      try {
        const next = await this.api.advance(request);
        this.checkAborted();
        this.submitFailures = 0;
        return next;
      } catch (error) {
        this.checkAborted();
        if (error instanceof RoomApiError && !error.retryable) throw error;
        this.submitFailures += 1;
        this.emit();
        await this.sleep(
          retryDelayMs(this.submitFailures, this.retryBaseMs, this.retryMaxMs, this.random()),
        );
        this.checkAborted();
      }
    }
  }

  /** 取一句语音并解码。取不到、解不了、太慢都返回 null，页面改成显示文字。 */
  private loadAudio(url: string): Promise<B | null> {
    let pending = this.audioCache.get(url);
    if (!pending) {
      pending = this.api
        .fetchAudio(url)
        .then((bytes) => this.audio.decode(bytes))
        .catch(() => {
          // 失败的不留在缓存里，重听或下次还能再试
          this.audioCache.delete(url);
          return null;
        });
      this.audioCache.set(url, pending);
    }
    return Promise.race([pending, this.sleep(this.audioTimeoutMs).then(() => null)]);
  }

  private dispatch(event: TurnEvent): void {
    if (!this.turn || this.aborted) return;
    const next = reduceTurn(this.turn, event, this.config);
    const changed = next !== this.turn;
    this.turn = next;
    if (changed && event.type !== "tick") this.onTurnEvent?.(event, next);
    if (changed || event.type === "tick") this.emit();
    if (this.turnWaiter?.until(next)) {
      const waiter = this.turnWaiter;
      this.turnWaiter = null;
      waiter.resolve();
    }
    this.maybeCheckPause();
  }

  /**
   * 候选人停顿了一小会儿：问服务端这段话说完没有。说完了就不用等满静音时长，没说完就多等一会儿。
   * 同一次停顿只问一次；问不到（接口报错、超时、服务端说不知道）就还按原来的静音时长算。
   */
  private maybeCheckPause(): void {
    const check = this.config.pauseCheck;
    const checkPause = this.api.checkPause?.bind(this.api);
    const step = this.step;
    const turn = this.turn;
    if (!check || !checkPause || !step || this.phase !== "listening") return;
    if (turn?.phase !== "listening" || turn.pausedAt !== null || turn.speaking) return;
    const { silenceSince, speechEndedAt, firstSpeechAt } = turn;
    if (silenceSince === null || speechEndedAt === null || firstSpeechAt === null) return;
    if (this.checkedSilenceSince === silenceSince || this.pauseChecks >= check.maxChecks) return;
    if (this.now() - silenceSince < check.afterMs) return;

    this.checkedSilenceSince = silenceSince;
    this.pauseChecks += 1;
    // 先把到这会儿的录像切一片传上去：服务端要拿这段声音转写
    this.onAnswerPaused?.();
    checkPause({
      seq: step.seq,
      utterance_key: step.utterance.key,
      playback_end_ms: this.playbackEnd,
      answer_start_ms: firstSpeechAt,
      answer_end_ms: speechEndedAt,
      ...(this.cutPlayedMs === null ? {} : { interrupted_played_ms: this.cutPlayedMs }),
    }).then(
      (verdict) => {
        if (verdict === "unknown" || this.step !== step) return;
        this.dispatch({ type: "pause_verdict", at: this.now(), verdict, silenceSince });
      },
      () => {
        // 问不到就算了，按原来的静音时长
      },
    );
  }

  private waitForTurn(until: (state: TurnState) => boolean): Promise<void> {
    if (this.aborted || (this.turn && until(this.turn))) return Promise.resolve();
    return new Promise((resolve) => {
      this.turnWaiter = { until, resolve };
    });
  }

  private sleep(ms: number): Promise<void> {
    if (this.aborted) return Promise.resolve();
    return new Promise((resolve) => {
      const wake = () => {
        clearTimeout(timer);
        this.abortWaiters.delete(wake);
        resolve();
      };
      const timer = setTimeout(wake, ms);
      this.abortWaiters.add(wake);
    });
  }

  private checkAborted(): void {
    if (this.aborted) throw new Aborted();
  }

  private startTicking(): void {
    this.stopTicking();
    this.ticker = setInterval(() => this.dispatch({ type: "tick", at: this.now() }), this.tickMs);
  }

  private stopTicking(): void {
    if (this.ticker !== null) clearInterval(this.ticker);
    this.ticker = null;
  }

  private setPhase(phase: RunnerPhase): void {
    this.phase = phase;
    this.emit();
  }

  private emit(): void {
    this.snapshot = this.buildSnapshot();
    for (const listener of this.listeners) listener(this.snapshot);
  }

  private buildSnapshot(): RunnerSnapshot {
    const listening =
      this.phase === "listening" && this.turn?.phase === "listening" ? this.turn : null;
    const view = listening ? viewTurn(listening, this.now(), this.config) : null;
    const vad = this.vad.snapshot();
    return {
      phase: this.phase,
      utterance: this.step?.utterance ?? null,
      expectsAnswer: this.step?.expects_answer ?? false,
      progress: this.step?.progress ?? null,
      turn: view,
      speaking: listening?.speaking ?? false,
      level: levelFromDb(vad.levelDb),
      canReplay: this.hasAudio && (view?.canReplay ?? false),
      paused: listening !== null && listening.pausedAt !== null,
      submitFailures: this.submitFailures,
      finishReason: this.finishReason,
      error: this.error,
    };
  }
}
