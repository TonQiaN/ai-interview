/**
 * 面试用的音频连接图。
 *
 *   麦克风 ──┬──> 音量计算（AudioWorklet）──> onLevel 回调（给静音检测用）
 *            └──> 混音输出 ──────────────┐
 *   面试官语音 ──┬──> 扬声器              ├──> mixedStream（给 MediaRecorder 录）
 *                └──> 混音输出 ──────────┘
 *
 * 这样录下来的音轨里候选人和面试官的声音都有，HR 回看时听得到完整的对话；
 * 麦克风本身不接扬声器，候选人听不到自己的回声。
 */
import levelProcessorUrl from "./level-processor.js?url";

export interface LevelFrame {
  /** 这一帧的均方根音量，0 到 1。 */
  rms: number;
  /** 这一帧的时长（毫秒）。 */
  frameMs: number;
}

export interface Playback {
  /** 这段音频的时长（毫秒）。 */
  durationMs: number;
  /** 播完（ended）或被 stop 打断（stopped）时兑现。 */
  done: Promise<"ended" | "stopped">;
  stop(): void;
}

export interface AudioGraphOptions {
  /** getUserMedia 拿到的流，至少带一条音轨。 */
  micStream: MediaStream;
  onLevel?: (frame: LevelFrame) => void;
}

/** 没有 AudioWorklet 时退回主线程定时读音量的间隔（毫秒）。 */
const FALLBACK_FRAME_MS = 20;

export class AudioGraph {
  private fallbackTimer: ReturnType<typeof setInterval> | null = null;
  private current: { source: AudioBufferSourceNode; stop: () => void } | null = null;

  private constructor(
    readonly context: AudioContext,
    private readonly micSource: MediaStreamAudioSourceNode,
    private readonly mixDestination: MediaStreamAudioDestinationNode,
    private readonly playbackBus: GainNode,
  ) {}

  static async create(options: AudioGraphOptions): Promise<AudioGraph> {
    const context = new AudioContext({ latencyHint: "interactive" });
    const micSource = context.createMediaStreamSource(options.micStream);
    const mixDestination = context.createMediaStreamDestination();
    // 人声用单声道就够，码率省一半
    mixDestination.channelCount = 1;
    const playbackBus = context.createGain();

    micSource.connect(mixDestination);
    playbackBus.connect(context.destination);
    playbackBus.connect(mixDestination);

    const graph = new AudioGraph(context, micSource, mixDestination, playbackBus);
    if (options.onLevel) await graph.attachLevelMeter(options.onLevel);
    return graph;
  }

  /** 混音后的流（一条音轨），交给 MediaRecorder。 */
  get mixedStream(): MediaStream {
    return this.mixDestination.stream;
  }

  /**
   * 让音频开始运转。浏览器要求第一次出声必须由用户的点击触发，
   * 所以要在「开始面试」「测试扬声器」这类按钮的点击处理里调用。
   */
  async resume(): Promise<void> {
    if (this.context.state !== "running") await this.context.resume();
  }

  /** 把一段音频文件的字节解码出来，供 play 使用。可以提前解码好。 */
  decode(data: ArrayBuffer): Promise<AudioBuffer> {
    // decodeAudioData 会接管传入的缓冲区，复制一份，调用方手里的还能用
    return this.context.decodeAudioData(data.slice(0));
  }

  /** 播放面试官的一句话。同一时刻只播一句，新的会打断旧的。 */
  play(buffer: AudioBuffer): Playback {
    this.current?.stop();

    const source = this.context.createBufferSource();
    source.buffer = buffer;
    source.connect(this.playbackBus);

    let settle: (result: "ended" | "stopped") => void = () => {};
    const done = new Promise<"ended" | "stopped">((resolve) => {
      settle = resolve;
    });
    let stopped = false;
    const stop = () => {
      if (stopped) return;
      stopped = true;
      try {
        source.stop();
      } catch {
        // 还没 start 或已经结束，忽略
      }
    };
    source.onended = () => {
      if (this.current?.source === source) this.current = null;
      source.disconnect();
      settle(stopped ? "stopped" : "ended");
    };

    this.current = { source, stop };
    source.start();
    return { durationMs: Math.round(buffer.duration * 1000), done, stop };
  }

  /** 生成一段提示音，用来测试扬声器。 */
  createTone(frequencyHz = 660, durationMs = 600): AudioBuffer {
    const { sampleRate } = this.context;
    const length = Math.round((sampleRate * durationMs) / 1000);
    const buffer = this.context.createBuffer(1, length, sampleRate);
    const samples = buffer.getChannelData(0);
    const fade = Math.min(length / 2, Math.round(sampleRate * 0.02));
    for (let i = 0; i < length; i += 1) {
      // 首尾各 20 毫秒淡入淡出，避免咔哒声
      const envelope = Math.min(1, i / fade, (length - 1 - i) / fade);
      samples[i] = 0.25 * envelope * Math.sin((2 * Math.PI * frequencyHz * i) / sampleRate);
    }
    return buffer;
  }

  async close(): Promise<void> {
    this.current?.stop();
    if (this.fallbackTimer !== null) clearInterval(this.fallbackTimer);
    this.micSource.disconnect();
    this.playbackBus.disconnect();
    if (this.context.state !== "closed") await this.context.close();
  }

  private async attachLevelMeter(onLevel: (frame: LevelFrame) => void): Promise<void> {
    // 有些浏览器不处理没接到输出端的节点，所以经过一个音量为 0 的节点接到扬声器
    const sink = this.context.createGain();
    sink.gain.value = 0;
    sink.connect(this.context.destination);

    if (this.context.audioWorklet) {
      try {
        await this.context.audioWorklet.addModule(levelProcessorUrl);
        const node = new AudioWorkletNode(this.context, "level-processor", {
          numberOfInputs: 1,
          numberOfOutputs: 1,
          channelCount: 1,
          channelCountMode: "explicit",
        });
        node.port.onmessage = (event: MessageEvent<LevelFrame>) => onLevel(event.data);
        this.micSource.connect(node);
        node.connect(sink);
        return;
      } catch {
        // 加载失败（旧 WebView、内容安全策略等），走下面的退路
      }
    }

    const analyser = this.context.createAnalyser();
    analyser.fftSize = 1024;
    this.micSource.connect(analyser);
    analyser.connect(sink);
    const samples = new Float32Array(analyser.fftSize);
    let last = performance.now();
    this.fallbackTimer = setInterval(() => {
      analyser.getFloatTimeDomainData(samples);
      let sumSquares = 0;
      for (const sample of samples) sumSquares += sample * sample;
      const now = performance.now();
      // 页面在后台时定时器会被降频，一帧可能代表很长时间；封顶，避免一帧就把静音计满
      const frameMs = Math.min(250, now - last);
      last = now;
      onLevel({ rms: Math.sqrt(sumSquares / samples.length), frameMs });
    }, FALLBACK_FRAME_MS);
  }
}
