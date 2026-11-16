/**
 * 一场面试在候选人浏览器里的全部状态：从打开链接到上传完录像。
 *
 * 页面只做两件事：订阅这里的快照来画界面，把按钮点击转成这里的方法调用。
 * 摄像头、音频、录制、上传、过程事件这些资源都由它创建和释放。
 */
import {
  ACTIVE_ELSEWHERE,
  HttpRoomApi,
  type RoomApi,
  RoomApiError,
  type RoomApiHooks,
} from "../lib/api/room-api";
import type { NextStep, RoomSession } from "../lib/api/room-types";
import {
  bindPageEvents,
  collectableSignals,
  EventCollector,
  type PageEventBinding,
  type PageTargets,
} from "../lib/events/collector";
import {
  type CapabilityReport,
  checkCapabilities,
  readBrowserEnvironment,
} from "../lib/flow/capabilities";
import { readInviteToken } from "../lib/flow/link";
import { measureUplinkKbps } from "../lib/flow/uplink";
import { InterviewRunner, levelFromDb, type RunnerSnapshot } from "../lib/interview/runner";
import { AudioGraph, type LevelFrame } from "../lib/media/audio-graph";
import { CaptureError, requestCapture, stopStream } from "../lib/media/capture";
import type { MediaErrorKind } from "../lib/media/errors";
import type { OutputHalt, OutputWatch } from "../lib/media/output-watch";
import { ScreenWakeLock } from "../lib/media/wake-lock";
import { recorderFactory } from "../lib/recording/create-recorder";
import { SegmentRecorder } from "../lib/recording/segment-recorder";
import { Timeline } from "../lib/timeline";
import {
  type ChunkStore,
  MemoryChunkStore,
  type RecordingTrack,
  sha256Hex,
} from "../lib/upload/chunk-store";
import { IndexedDbChunkStore } from "../lib/upload/idb-chunk-store";
import { discardSession, discardStale } from "../lib/upload/leftovers";
import { closeOrphanSegments } from "../lib/upload/orphans";
import { OwnedChunkStore } from "../lib/upload/owned-chunk-store";
import { announcePage, isPageOpen } from "../lib/upload/page-presence";
import { SpillChunkStore } from "../lib/upload/spill-chunk-store";
import { UploadEventTracker } from "../lib/upload/upload-events";
import { retryDelayMs, Uploader, type UploadStatus } from "../lib/upload/uploader";
import { EnergyVad } from "../lib/vad/energy-vad";

export type SessionStage =
  | "loading"
  /** 没法面试：链接有问题、浏览器不支持等。看 problem。 */
  | "problem"
  | "welcome"
  | "consent"
  | "device"
  | "ready"
  | "interview"
  /** 面试结束，正在把剩下的录像传完。 */
  | "finishing"
  | "done";

export type ProblemKind =
  /** 地址里没有令牌。 */
  | "missing_link"
  | "invalid_link"
  | "expired"
  | "revoked"
  /** 浏览器缺少面试需要的能力。 */
  | "unsupported"
  /** 连不上服务器。可以重试。 */
  | "network"
  /** 别的错误。 */
  | "error";

export interface DeviceState {
  permission: "idle" | "requesting" | "granted" | "failed";
  /** 申请失败的原因。 */
  error: MediaErrorKind | null;
  /** 有没有拿到摄像头画面。 */
  hasVideo: boolean;
  /** 麦克风有没有听到候选人说话。 */
  micHeard: boolean;
  speaker: "idle" | "playing" | "confirmed";
}

/** 上行网速的检测。量不出来（failed）或者偏慢都不拦候选人。 */
export interface NetworkState {
  status: "idle" | "testing" | "measured" | "failed";
  /** 量出来的上行（千比特每秒）。 */
  uplinkKbps: number | null;
}

export interface SessionSnapshot {
  stage: SessionStage;
  problem: ProblemKind | null;
  capability: CapabilityReport | null;
  room: RoomSession | null;
  device: DeviceState;
  network: NetworkState;
  /** 有操作正在进行（按钮显示加载中）。 */
  busy: boolean;
  /** 上一次操作失败的提示。 */
  actionError: string | null;
  runner: RunnerSnapshot | null;
  upload: UploadStatus | null;
  /** 面试中摄像头或麦克风断了。 */
  interrupted: boolean;
  /** 面试中声音被系统停掉了，自己恢复不了，要候选人点一下「继续面试」。 */
  audioHalted: boolean;
  /** 录像分片只能放在内存里（没有 IndexedDB），刷新会丢。 */
  volatileStorage: boolean;
  /** 面试是怎么结束的。`taken_over` 是面试没有结束，只是换到另一个页面里继续了。 */
  finishReason: string | null;
}

/** 面试换到另一个页面里继续了，这个页面停了下来。 */
export const TAKEN_OVER = "taken_over";

const INITIAL_DEVICE: DeviceState = {
  permission: "idle",
  error: null,
  hasVideo: false,
  micHeard: false,
  speaker: "idle",
};

export interface InterviewSessionOptions {
  /** location.hash。 */
  hash: string;
  /** 测试时可以换掉接口实现。 */
  createApi?: (token: string, hooks: RoomApiHooks) => RoomApi;
  /** 这个页面的标识。默认随机生成。 */
  pageId?: string;
}

export class InterviewSession {
  private readonly hash: string;
  private readonly createApi: (token: string, hooks: RoomApiHooks) => RoomApi;
  private readonly listeners = new Set<() => void>();
  private readonly levelListeners = new Set<(level: number) => void>();
  private readonly vad = new EnergyVad();

  private snapshot: SessionSnapshot = {
    stage: "loading",
    problem: null,
    capability: null,
    room: null,
    device: INITIAL_DEVICE,
    network: { status: "idle", uplinkKbps: null },
    busy: false,
    actionError: null,
    runner: null,
    upload: null,
    interrupted: false,
    audioHalted: false,
    volatileStorage: false,
    finishReason: null,
  };

  /** 这个页面的标识，每次载入换一个：服务端靠它分清是哪个页面在面试，本机的录像队列靠它分清一段是谁录的。 */
  private readonly pageId: string;
  private api: RoomApi | null = null;
  private sessionKey = "";
  private store: ChunkStore | null = null;
  private uploader: Uploader | null = null;
  private collector: EventCollector | null = null;
  private pageEvents: PageEventBinding | null = null;
  private timeline: Timeline | null = null;
  private stream: MediaStream | null = null;
  private graph: AudioGraph | null = null;
  private recorder: SegmentRecorder | null = null;
  private runner: InterviewRunner<AudioBuffer> | null = null;
  private wakeLock: ScreenWakeLock | null = null;
  private outputWatch: OutputWatch | null = null;
  /** 声音是什么时候被系统停掉的（performance.now()）；没停是 null。 */
  private outputHaltedAt: number | null = null;
  /** 面试因为什么被按住了。全都放开了才接着走。 */
  private readonly holds = new Set<"device" | "audio">();
  /** 面试是什么时候结束的（performance.now()）；这次打开页面时面试已经结束是 null。 */
  private finishedAt: number | null = null;
  private readonly cleanups: Array<() => void> = [];
  private trackCleanups: Array<() => void> = [];
  private disposed = false;

  constructor(options: InterviewSessionOptions) {
    this.hash = options.hash;
    this.createApi = options.createApi ?? ((token, hooks) => new HttpRoomApi({ token, ...hooks }));
    this.pageId = options.pageId ?? crypto.randomUUID();
  }

  getSnapshot = (): SessionSnapshot => this.snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** 麦克风音量（0 到 1），每 20 毫秒一次。音量条自己订阅，不走快照，免得整页跟着重画。 */
  subscribeLevel = (listener: (level: number) => void): (() => void) => {
    this.levelListeners.add(listener);
    return () => this.levelListeners.delete(listener);
  };

  /** 摄像头的流，给 <video> 预览用。 */
  get cameraStream(): MediaStream | null {
    return this.stream && this.stream.getVideoTracks().length > 0 ? this.stream : null;
  }

  /** 面试已经进行了多少毫秒；还没开始是 null。 */
  elapsedMsNow = (): number | null => this.timeline?.now() ?? null;

  // ── 打开链接 ──

  async load(): Promise<void> {
    this.patch({ stage: "loading", problem: null, actionError: null });

    const token = readInviteToken(this.hash);
    if (!token) return this.fail("missing_link");

    const capability = checkCapabilities(readBrowserEnvironment());
    this.patch({ capability });
    if (!capability.ok) return this.fail("unsupported");

    if (!this.api) {
      this.api = this.createApi(token, {
        pageId: this.pageId,
        onActiveElsewhere: this.handleActiveElsewhere,
      });
      // 本地队列按面试区分。用令牌的摘要做键，令牌本身不落盘
      this.sessionKey = (
        await sha256Hex(new TextEncoder().encode(token).buffer as ArrayBuffer)
      ).slice(0, 32);
    }
    const api = this.api;

    let room: RoomSession;
    try {
      room = await api.getSession();
    } catch (error) {
      return this.fail(problemOf(error));
    }
    if (this.disposed) return;

    await this.setUpBackground(api);
    this.collector?.record("page_opened", {
      stage: room.stage,
      in_app_browser: capability.inAppBrowser,
      recording_mime: capability.recordingMime,
      // 这个浏览器里哪几类过程事件采得到：报告据此区分「没发生」和「采不到」
      signals: collectableSignals(this.pageTargets(), navigator),
    });

    if (room.stage === "finished") {
      // 面试已经结束。上次如果有没传完的录像，这次接着传
      this.patch({ room, stage: "finishing", finishReason: room.end_reason ?? null });
      void this.drainUploads();
      return;
    }
    this.patch({ room, stage: "welcome" });
  }

  /** 事件收集和上传队列：整个页面生命期只建一次。 */
  private async setUpBackground(api: RoomApi): Promise<void> {
    if (this.collector) return;

    const collector = new EventCollector({
      send: (events, options) => api.postEvents(events, options),
      timelineNow: () => this.timeline?.now() ?? null,
    });
    collector.start();
    this.collector = collector;
    const pageEvents = bindPageEvents(collector, this.pageTargets());
    this.pageEvents = pageEvents;
    this.cleanups.push(
      () => collector.stop(),
      () => pageEvents.unbind(),
    );

    let store: ChunkStore;
    try {
      // 面试到一半本机存不下了（磁盘满了、配额用完了），写不进去的分片先留在内存里接着传
      store = new SpillChunkStore(await IndexedDbChunkStore.open(), {
        onSpill: (error) =>
          collector.record("recording_error", { message: error.name, kept_in_memory: true }),
      });
    } catch {
      // 隐私模式等情况下 IndexedDB 不可用：分片放内存，能录能传，只是刷新会丢没传完的
      store = new MemoryChunkStore();
      this.patch({ volatileStorage: true });
    }
    // 同一个浏览器里别的标签页也在用这份队列：先说一声这个页面开着，再只管归自己的那些段
    this.cleanups.push(announcePage(this.pageId));
    store = new OwnedChunkStore(store, { pageId: this.pageId, isPageOpen });
    this.store = store;
    // 别的面试在这台设备上留下的旧录像已经传不上去了，顺手清掉
    await discardStale(store, { keep: this.sessionKey, now: Date.now() }).catch(() => {});

    const uploader = new Uploader({
      sessionKey: this.sessionKey,
      store,
      api,
      isOnline: () => navigator.onLine,
    });
    this.uploader = uploader;
    // 录像传不动了、又传动了，各记一条过程事件
    const uploadEvents = new UploadEventTracker({
      record: (kind, payload) => collector.record(kind, payload),
    });
    this.cleanups.push(
      uploader.subscribe((upload) => {
        uploadEvents.observe(upload);
        this.patch({ upload });
      }),
      () => uploader.stop(),
    );
    const onOnline = () => {
      void this.closeOrphans();
      uploader.notify();
    };
    window.addEventListener("online", onOnline);
    this.cleanups.push(() => window.removeEventListener("online", onOnline));

    // 上次页面留下的分片（刷新、崩溃）接着传
    await this.closeOrphans();
    uploader.start();
  }

  private async closeOrphans(): Promise<void> {
    if (!this.store || !this.api) return;
    // 正在录的那一段不算遗留
    if (this.recorder) return;
    try {
      await closeOrphanSegments(this.store, this.sessionKey, this.api);
      this.uploader?.notify();
    } catch {
      // 本地存储读不了就算了，不影响面试
    }
  }

  // ── 进入流程 ──

  /** 欢迎页点「开始准备」。 */
  proceedFromWelcome(): void {
    const room = this.snapshot.room;
    if (!room) return;
    this.patch({ stage: room.consent.agreed ? "device" : "consent", actionError: null });
    if (room.consent.agreed) void this.checkNetwork();
  }

  async agree(agreedKeys: string[]): Promise<void> {
    const { room } = this.snapshot;
    if (!room || !this.api) return;
    await this.action(async () => {
      await this.api?.postConsent(room.consent.version, agreedKeys);
      this.patch({
        room: { ...room, consent: { ...room.consent, agreed: true } },
        stage: "device",
      });
      void this.checkNetwork();
    });
  }

  /**
   * 量一次上行网速。进入设备检测时自己开始，候选人也可以点「重新测速」。
   * 量的时候候选人可以照常检查摄像头和麦克风。
   */
  async checkNetwork(): Promise<void> {
    const api = this.api;
    if (!api || this.snapshot.network.status === "testing") return;
    this.patch({ network: { status: "testing", uplinkKbps: null } });
    try {
      const uplinkKbps = await measureUplinkKbps({ send: (bytes) => api.probeUplink(bytes) });
      this.patch({ network: { status: "measured", uplinkKbps } });
    } catch {
      this.patch({ network: { status: "failed", uplinkKbps: null } });
    }
  }

  private pageTargets(): PageTargets {
    return { document, window, mediaDevices: navigator.mediaDevices };
  }

  /** 申请摄像头和麦克风。必须在按钮点击里调用。 */
  async requestDevices(): Promise<void> {
    const { room } = this.snapshot;
    if (!room) return;
    this.patch({ device: { ...INITIAL_DEVICE, permission: "requesting" }, actionError: null });
    try {
      const stream = await this.capture(room);
      this.stream = stream;
      if (this.graph) {
        this.graph.replaceMic(stream);
      } else {
        this.graph = await AudioGraph.create({ micStream: stream, onLevel: this.handleLevel });
      }
      await this.graph.resume();
      this.watchTracks(stream);
      // 授权之后浏览器才给出完整的设备清单，重新记一次，免得之后把这个差别当成换了设备
      void this.pageEvents?.rebaselineDevices();
      this.patch({
        device: {
          permission: "granted",
          error: null,
          hasVideo: stream.getVideoTracks().length > 0,
          micHeard: false,
          speaker: "idle",
        },
      });
    } catch (error) {
      const kind = error instanceof CaptureError ? error.kind : "unknown";
      this.collector?.record("device_check", { result: "failed", error: kind });
      this.patch({ device: { ...INITIAL_DEVICE, permission: "failed", error: kind } });
    }
  }

  private async capture(room: RoomSession): Promise<MediaStream> {
    const settings = {
      width: room.recording.width,
      height: room.recording.height,
      frameRate: room.recording.frame_rate,
    };
    try {
      return await requestCapture({ video: true, ...settings });
    } catch (error) {
      // 这场面试不强制摄像头时，没有摄像头就只用麦克风
      const noCamera =
        error instanceof CaptureError &&
        (error.kind === "device_not_found" || error.kind === "constraints_unsatisfied");
      if (room.camera_required || !noCamera) throw error;
      return requestCapture({ video: false });
    }
  }

  /** 放一段提示音，让候选人确认听得到。 */
  async playTestSound(): Promise<void> {
    const graph = this.graph;
    if (!graph || this.snapshot.device.speaker === "playing") return;
    await graph.resume();
    const before = this.snapshot.device.speaker;
    this.patch({ device: { ...this.snapshot.device, speaker: "playing" } });
    await graph.play(graph.createTone(660, 900)).done;
    const after = this.getSnapshot().device;
    if (after.speaker === "playing") this.patch({ device: { ...after, speaker: before } });
  }

  confirmSpeaker(): void {
    this.patch({ device: { ...this.snapshot.device, speaker: "confirmed" } });
  }

  /** 设备检测做完，进入「准备开始」。 */
  finishDeviceCheck(): void {
    const { device, capability, network } = this.snapshot;
    if (device.permission !== "granted") return;
    // 检测结果只是给 HR 排查问题用的，发不出去不拦候选人
    void this.api
      ?.postDeviceCheck({
        camera: device.hasVideo ? "ok" : "skipped",
        microphone: device.micHeard ? "ok" : "failed",
        speaker: device.speaker === "confirmed" ? "ok" : "skipped",
        uplink_kbps: network.uplinkKbps,
        recording_mime: capability?.recordingMime ?? null,
        user_agent: navigator.userAgent,
      })
      .catch(() => {});
    this.patch({ stage: "ready", actionError: null });
  }

  // ── 面试 ──

  /** 「开始面试」。面试已经开始过（刷新后回来）时，服务端会返回当前该播的那一步。 */
  async begin(): Promise<void> {
    const { room } = this.snapshot;
    const { api, graph, stream } = this;
    if (!room || !api || !graph || !stream) return;

    await this.action(async () => {
      await graph.resume();
      const first = await api.start();
      this.timeline = new Timeline(
        first.action === "say" ? first.progress.elapsed_ms : (room.elapsed_ms ?? 0),
      );
      this.collector?.record("interview_started", { resumed: room.stage === "in_progress" });

      await this.startRecording();
      void this.keepScreenOn();
      window.addEventListener("beforeunload", this.warnBeforeUnload);
      this.cleanups.push(() => window.removeEventListener("beforeunload", this.warnBeforeUnload));

      const runner = new InterviewRunner<AudioBuffer>({
        api,
        audio: graph,
        now: () => this.timeline?.now() ?? 0,
        vad: this.vad,
        onTurnEvent: (event) => {
          if (event.type === "user_replay" || event.type === "user_continue") {
            this.collector?.record(
              event.type === "user_replay" ? "question_replayed" : "answer_continued",
            );
          }
        },
        // 答完一题马上切一片录像传上去：服务端要用这段声音转写，判断接下来问什么
        onAnswerEnded: () => this.recorder?.flush(),
        // 停顿时也切一片：服务端趁这会儿先转写、先想怎么接
        onAnswerPaused: () => this.recorder?.flush(),
      });
      this.runner = runner;
      this.cleanups.push(
        runner.subscribe((snapshot) => this.patch({ runner: snapshot })),
        () => runner.stop(),
      );
      this.watchOutput(graph);
      this.patch({
        stage: "interview",
        runner: runner.getSnapshot(),
        room: { ...room, stage: "in_progress" },
      });

      void this.runInterview(runner, first);
    });
  }

  private async runInterview(runner: InterviewRunner<AudioBuffer>, first: NextStep): Promise<void> {
    try {
      const reason = await runner.run(first);
      if (reason !== null) await this.finish(reason, false);
    } catch (error) {
      // 链接被撤销、面试被取消：停止录制。已经录下的服务端不再收，收尾时从本机删掉
      this.patch({ problem: problemOf(error) });
      await this.finish("aborted", false);
    }
  }

  /**
   * 服务端说面试正在另一个页面里进行（同一条链接又在别处打开，并且在那边点了继续）。
   * 这个页面正在面试的话就停下来：不再播、不再录，把已经录下的传完。面试本身没有结束。
   */
  private handleActiveElsewhere = (): void => {
    if (this.snapshot.stage !== "interview") return;
    this.runner?.stop();
    void this.finish(TAKEN_OVER, false);
  };

  /** 面试中不让屏幕自己熄灭。要在按钮点击里调用；申请不到不影响面试。 */
  private async keepScreenOn(): Promise<void> {
    this.wakeLock ??= new ScreenWakeLock(
      "wakeLock" in navigator ? navigator.wakeLock : undefined,
      document,
    );
    const lock = this.wakeLock;
    const held = await lock.acquire();
    // 记下来：屏幕中途熄灭导致录像断掉时，HR 能看出是不是因为这个
    this.collector?.record("screen_wake_lock", { supported: lock.supported, held });
  }

  private async startRecording(): Promise<void> {
    const { store, stream, graph } = this;
    const room = this.snapshot.room;
    if (!store || !stream || !graph || !room) return;
    const track: RecordingTrack = stream.getVideoTracks().length > 0 ? "camera" : "microphone";
    const recorder = new SegmentRecorder({
      sessionKey: this.sessionKey,
      track,
      store,
      createRecorder: recorderFactory(stream, graph.mixedStream, {
        videoBitsPerSecond: room.recording.video_bits_per_second,
        audioBitsPerSecond: room.recording.audio_bits_per_second,
      }),
      timesliceMs: room.recording.timeslice_ms,
      timelineNow: () => this.timeline?.now() ?? 0,
      onStored: () => this.uploader?.notify(),
      onError: (error) => this.collector?.record("recording_error", { message: error.name }),
    });
    this.recorder = recorder;
    await recorder.start();
    this.uploader?.notify();
  }

  // 面试中的按钮
  answerDone = (): void => this.runner?.done();
  continueAnswer = (): void => this.runner?.continueAnswer();
  replayQuestion = (): void => this.runner?.replay();
  readyToAnswer = (): void => this.runner?.ready();

  /** 候选人主动结束面试。 */
  async leave(): Promise<void> {
    this.runner?.stop();
    await this.finish("candidate_left", true);
  }

  /** 设备断了之后点「重新连接」。必须在按钮点击里调用。 */
  async reconnectDevices(): Promise<void> {
    const { room } = this.snapshot;
    if (!room || !this.graph) return;
    await this.action(async () => {
      const old = this.stream;
      const previous = this.recorder;
      this.recorder = null;
      await previous?.stop();
      stopStream(old);

      const stream = await this.capture(room);
      this.stream = stream;
      this.graph?.replaceMic(stream);
      await this.graph?.resume();
      this.watchTracks(stream);
      await this.startRecording();
      this.collector?.record("device_recovered");
      this.patch({ interrupted: false });
      this.release("device");
    });
  }

  /** 声音被系统停掉之后点「继续面试」。必须在按钮点击里调用：有的手机只在点击里才让声音恢复。 */
  async resumeAudio(): Promise<void> {
    const watch = this.outputWatch;
    if (!watch) return;
    await this.action(async () => {
      if (!(await watch.revive())) this.patch({ actionError: "audio_halted" });
    });
  }

  /** 面试开始后盯着声音有没有被系统停掉。 */
  private watchOutput(graph: AudioGraph): void {
    const watch = graph.watchOutput(this.handleOutputChange);
    this.outputWatch = watch;
    // 切回页面时系统多半已经肯让声音恢复了，不用等候选人点
    const onVisible = () => {
      if (document.visibilityState === "visible" && this.outputHaltedAt !== null) {
        void this.reviveOutput();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    this.cleanups.push(
      () => watch.stop(),
      () => document.removeEventListener("visibilitychange", onVisible),
    );
  }

  private handleOutputChange = (halt: OutputHalt | null): void => {
    if (this.snapshot.stage !== "interview") return;
    if (halt !== null) {
      if (this.outputHaltedAt !== null) return;
      this.outputHaltedAt = performance.now();
      // 声音停着的这一段，录像里没有声音，面试官的话也播不出来
      this.collector?.record("recording_gap", { device: "audio_output", cause: halt });
      this.hold("audio");
      void this.reviveOutput();
    } else if (this.outputHaltedAt !== null) {
      this.collector?.record("device_recovered", {
        device: "audio_output",
        duration_ms: Math.round(performance.now() - this.outputHaltedAt),
      });
      this.outputHaltedAt = null;
      this.patch({ audioHalted: false, actionError: null });
      this.release("audio");
    }
  };

  /** 先自己试着恢复；不行就让候选人点按钮。 */
  private async reviveOutput(): Promise<void> {
    const revived = await this.outputWatch?.revive();
    if (!revived && this.outputHaltedAt !== null && this.snapshot.stage === "interview") {
      this.patch({ audioHalted: true });
    }
  }

  /** 按住面试：计时停下，下一句先不播。 */
  private hold(reason: "device" | "audio"): void {
    const first = this.holds.size === 0;
    this.holds.add(reason);
    if (first) this.runner?.pause();
  }

  private release(reason: "device" | "audio"): void {
    if (!this.holds.delete(reason)) return;
    if (this.holds.size === 0) this.runner?.resume();
  }

  private watchTracks(stream: MediaStream): void {
    for (const off of this.trackCleanups) off();
    this.trackCleanups = [];
    for (const track of stream.getTracks()) {
      const onEnded = () => this.handleInterruption(track.kind, "ended");
      // 手机来电话时系统会把麦克风静音，挂断后恢复
      const onMute = () => track.kind === "audio" && this.handleInterruption(track.kind, "muted");
      const onUnmute = () => {
        if (track.kind !== "audio" || !this.snapshot.interrupted) return;
        this.collector?.record("device_recovered");
        this.patch({ interrupted: false });
        this.release("device");
      };
      track.addEventListener("ended", onEnded);
      track.addEventListener("mute", onMute);
      track.addEventListener("unmute", onUnmute);
      this.trackCleanups.push(() => {
        track.removeEventListener("ended", onEnded);
        track.removeEventListener("mute", onMute);
        track.removeEventListener("unmute", onUnmute);
      });
    }
  }

  private handleInterruption(kind: string, cause: "ended" | "muted"): void {
    if (this.snapshot.stage === "interview") {
      this.collector?.record("recording_gap", { device: kind, cause });
      this.hold("device");
      this.patch({ interrupted: true });
    } else if (this.snapshot.stage === "device" || this.snapshot.stage === "ready") {
      if (cause !== "ended") return;
      // 检测阶段设备没了：回到检测页重新来
      this.patch({
        stage: "device",
        device: { ...INITIAL_DEVICE, permission: "failed", error: "device_not_found" },
      });
    }
  }

  // ── 结束 ──

  private async finish(reason: string, notifyServer: boolean): Promise<void> {
    if (this.snapshot.stage === "finishing" || this.snapshot.stage === "done") return;
    this.finishedAt = performance.now();
    this.patch({
      stage: "finishing",
      finishReason: reason,
      interrupted: false,
      audioHalted: false,
    });
    // 换到别的页面继续的不算结束，另记一种事件：报告里能看出来中途换过页面
    if (reason === TAKEN_OVER) this.collector?.record("page_taken_over");
    else this.collector?.record("interview_finished", { reason });
    this.outputWatch?.stop();
    this.outputWatch = null;
    this.outputHaltedAt = null;
    this.holds.clear();

    const recorder = this.recorder;
    this.recorder = null;
    await recorder?.stop().catch(() => {});
    void this.wakeLock?.release();
    for (const off of this.trackCleanups) off();
    this.trackCleanups = [];
    stopStream(this.stream);
    this.stream = null;
    await this.graph?.close().catch(() => {});
    this.graph = null;

    if (notifyServer) await this.notifyFinished();
    await this.drainUploads();
  }

  /** 告诉服务端面试结束了。断网时一直重试到成功，或者链接已经失效。 */
  private async notifyFinished(): Promise<void> {
    for (let failures = 1; !this.disposed; failures += 1) {
      try {
        await this.api?.finish("candidate_left");
        return;
      } catch (error) {
        // 这个页面已经不是在面试的那个了，结束不了面试：面试还在另一个页面里继续
        if (error instanceof RoomApiError && error.code === ACTIVE_ELSEWHERE) {
          this.collector?.record("page_taken_over");
          this.patch({ finishReason: TAKEN_OVER });
        }
        if (error instanceof RoomApiError && !error.retryable) return;
        await new Promise((resolve) =>
          setTimeout(resolve, retryDelayMs(failures, 1_000, 10_000, Math.random())),
        );
      }
    }
  }

  private async drainUploads(): Promise<void> {
    try {
      await this.closeOrphans();
      await this.uploader?.drain();
      // 面试结束后候选人等了多久才传完。刷新后回来补传的那一次不知道从哪算起，不带等待时间
      this.collector?.record("upload_finished", {
        uploaded_bytes: this.uploader?.getStatus().uploadedBytes ?? 0,
        wait_ms: this.finishedAt === null ? null : Math.round(performance.now() - this.finishedAt),
      });
      await this.collector?.flush();
      window.removeEventListener("beforeunload", this.warnBeforeUnload);
      this.patch({ stage: "done", actionError: null });
    } catch (error) {
      const dead = await this.deadLinkProblem();
      if (dead === null) {
        this.patch({ actionError: error instanceof Error ? error.message : String(error) });
        return;
      }
      // 链接已经不能用了，剩下的录像谁也传不上去：不让候选人一直点「重试」
      window.removeEventListener("beforeunload", this.warnBeforeUnload);
      if (this.snapshot.finishReason === "aborted") {
        // 面试进行到一半链接失效的：告诉候选人面试中止了
        await this.discardLocalRecordings();
        this.patch({ stage: "done", problem: dead, actionError: null });
      } else {
        this.fail(dead);
      }
    }
  }

  /** 录像传不上去时问一下服务端：是不是这条链接已经不能用了。还能用、或者问不到，返回 null。 */
  private async deadLinkProblem(): Promise<ProblemKind | null> {
    if (!this.api) return null;
    try {
      await this.api.getSession();
      return null;
    } catch (error) {
      const problem = problemOf(error);
      return isDeadLink(problem) ? problem : null;
    }
  }

  /**
   * 链接再也用不了了（过期、被撤销、面试被取消或删除）：留在本机的录像谁也传不上去，删掉。
   * 录像里是候选人的脸和声音，不该留在可能是借来的设备上。
   */
  private async discardLocalRecordings(): Promise<void> {
    this.uploader?.stop();
    try {
      this.store ??= await IndexedDbChunkStore.open();
      await discardSession(this.store, this.sessionKey);
      await discardStale(this.store, { keep: this.sessionKey, now: Date.now() });
    } catch {
      // 本地存储打不开，也就没有东西留在里面
    }
  }

  /** 录像上传失败后点「重试」。 */
  async retryUpload(): Promise<void> {
    this.patch({ actionError: null });
    this.uploader?.start();
    await this.drainUploads();
  }

  /** 页面卸载。 */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    void this.collector?.flush({ keepalive: true });
    this.runner?.stop();
    void this.wakeLock?.release();
    for (const off of this.trackCleanups) off();
    for (const off of this.cleanups.splice(0)) off();
    stopStream(this.stream);
    void this.graph?.close().catch(() => {});
  }

  // ── 内部 ──

  private handleLevel = (frame: LevelFrame): void => {
    if (this.runner) {
      this.runner.handleLevel(frame);
    } else {
      // 设备检测阶段：听到候选人说话就算麦克风通过
      const event = this.vad.process(frame.rms, frame.frameMs);
      if (
        event?.type === "speech_start" &&
        !this.snapshot.device.micHeard &&
        this.snapshot.device.speaker !== "playing"
      ) {
        this.patch({ device: { ...this.snapshot.device, micHeard: true } });
      }
    }
    if (this.levelListeners.size > 0) {
      const level = levelFromDb(this.vad.snapshot().levelDb);
      for (const listener of this.levelListeners) listener(level);
    }
  };

  private warnBeforeUnload = (event: BeforeUnloadEvent): void => {
    if (this.snapshot.stage !== "interview" && this.snapshot.stage !== "finishing") return;
    event.preventDefault();
  };

  private async action(run: () => Promise<void>): Promise<void> {
    if (this.snapshot.busy) return;
    this.patch({ busy: true, actionError: null });
    try {
      await run();
    } catch (error) {
      if (
        error instanceof RoomApiError &&
        !error.retryable &&
        error.kind !== "conflict" &&
        error.kind !== "rejected"
      ) {
        this.fail(problemOf(error));
      } else if (error instanceof CaptureError) {
        this.patch({ actionError: `capture:${error.kind}` });
      } else {
        this.patch({
          actionError:
            error instanceof RoomApiError && error.kind === "network"
              ? "network"
              : messageOf(error),
        });
      }
    } finally {
      this.patch({ busy: false });
    }
  }

  private fail(problem: ProblemKind): void {
    this.patch({ stage: "problem", problem });
    if (isDeadLink(problem)) void this.discardLocalRecordings();
  }

  private patch(patch: Partial<SessionSnapshot>): void {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
}

/** 服务端明确说这条链接不能用了。网络不通、服务端出错不算：那些等一等还能接着传。 */
function isDeadLink(problem: ProblemKind): boolean {
  return problem === "invalid_link" || problem === "expired" || problem === "revoked";
}

function problemOf(error: unknown): ProblemKind {
  if (!(error instanceof RoomApiError)) return "error";
  switch (error.kind) {
    case "invalid_link":
      return "invalid_link";
    case "expired":
      return "expired";
    case "revoked":
      return "revoked";
    case "network":
    case "rate_limited":
      return "network";
    default:
      return "error";
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
