/**
 * 逐题面试的编排：把「播一句 → 听回答 → 上报 → 拿下一句」这个循环跑起来。
 *
 * 它不直接碰浏览器接口：音频播放、接口调用、时钟都从外面传进来，所以能在 Node 里用假的实现测。
 * 画面通过 subscribe 拿快照，按钮点击调 done / continueAnswer / replay / ready。
 *
 * 断网时 advance 会一直重试（录制不受影响，分片先存在本地），候选人看到的是「正在重新连接」。
 * 只有链接失效、面试被取消这类重试没用的错误才会让 run 失败。
 */
import { RoomApiError } from "../api/room-api";
import type { AdvanceRequest, AnswerEndReason, AnswerSettings, NextStep, Progress, Utterance } from "../api/room-types";
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
  play(buffer: B): Playback;
}

export interface RunnerApi {
  advance(request: AdvanceRequest): Promise<NextStep>;
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

type SayStep = Extract<NextStep, { action: "say" }>;

const END_REASON: Record<TurnEndReason, AnswerEndReason> = {
  silence: "silence",
  manual: "button",
  no_speech: "no_speech",
  max_duration: "timeout",
};

/** 把服务端给的作答设置换成状态机的配置。 */
export function turnConfigFor(answer: AnswerSettings | null, base: TurnConfig = DEFAULT_TURN_CONFIG): TurnConfig {
  if (!answer) return base;
  const silenceMs = Math.max(1_000, Math.round(answer.silence_sec * 1000));
  const maxAnswerMs = Math.max(10_000, Math.round(answer.max_sec * 1000));
  return {
    ...base,
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
      this.error = error instanceof RoomApiError ? error : new RoomApiError("rejected", String(error));
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
    if (!event || this.turn?.phase !== "listening" || this.turn.pausedAt !== null) return;
    this.dispatch({ type: event.type, at: this.now() });
  };

  // ── 内部 ──

  private async runUtterance(step: SayStep): Promise<AdvanceRequest> {
    this.step = step;
    this.config = turnConfigFor(step.answer);
    this.turn = null;
    this.hasAudio = false;
    this.setPhase("preparing");
    for (const url of step.preload_audio_urls ?? []) void this.loadAudio(url);

    const buffer = step.utterance.audio_url ? await this.loadAudio(step.utterance.audio_url) : null;
    this.checkAborted();
    this.hasAudio = buffer !== null;

    let playbackStart: number | null = null;
    let playbackEnd = 0;
    this.turn = startTurn(this.now());

    for (;;) {
      if (buffer) {
        playbackStart ??= this.now();
        this.setPhase("asking");
        this.playback = this.audio.play(buffer);
        await this.playback.done;
        this.playback = null;
        this.checkAborted();
        playbackEnd = this.now();
        if (step.expects_answer) this.dispatch({ type: "question_ended", at: playbackEnd });
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
        playbackEnd = this.now();
      }

      if (!step.expects_answer) {
        return {
          utterance_key: step.utterance.key,
          playback_start_ms: playbackStart,
          playback_end_ms: playbackEnd,
          answer_start_ms: null,
          answer_end_ms: null,
          end_reason: null,
          replays: 0,
        };
      }

      // 面试官的声音刚停，之前的音量状态不能带进来
      this.vad.reset();
      this.setPhase("listening");
      this.startTicking();
      await this.waitForTurn((state) => state.phase === "ended" || state.phase === "asking");
      this.stopTicking();
      this.checkAborted();

      const turn = this.turn as TurnState;
      if (turn.phase !== "ended") continue; // 重听题目

      return {
        utterance_key: step.utterance.key,
        playback_start_ms: playbackStart,
        playback_end_ms: playbackEnd,
        answer_start_ms: turn.firstSpeechAt,
        answer_end_ms: turn.lastSpeechEndAt,
        end_reason: END_REASON[turn.reason],
        replays: turn.replaysUsed,
      };
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
        await this.sleep(retryDelayMs(this.submitFailures, this.retryBaseMs, this.retryMaxMs, this.random()));
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
    const listening = this.phase === "listening" && this.turn?.phase === "listening" ? this.turn : null;
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
