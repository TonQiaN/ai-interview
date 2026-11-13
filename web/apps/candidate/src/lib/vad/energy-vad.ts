/**
 * 基于能量的语音活动检测（VAD）。
 *
 * 纯逻辑：输入一帧一帧的音量（RMS）和这一帧的时长，输出「开始说话 / 停止说话」事件。
 * 不碰浏览器接口，方便单测；浏览器里的采集在 vad-worklet.ts 和 mic-vad.ts。
 *
 * 做法：
 * - 持续估计环境底噪：取最近几秒里出现过的最低音量。人说话时字和字之间总有接近底噪的间隙，
 *   所以说话不会把底噪抬高；风扇、空调这类持续的声音没有间隙，几秒后就被当成新的底噪。
 *   安静的房间和有背景噪声的房间都能自适应。
 * - 音量超过「底噪 + 进入余量」并持续够久才算开始说话（挡住咳嗽、敲桌子这类瞬时声音）。
 * - 音量回落到「底噪 + 退出余量」以下并持续够久才算停止说话（词与词之间的短停顿不算）。
 *
 * 这只回答「此刻有没有人在出声」。连续静音多久算答完，由 turn-machine.ts 决定。
 */

export interface EnergyVadConfig {
  /** 进入说话状态：音量要比底噪高出多少 dB。 */
  enterMarginDb: number;
  /** 退出说话状态：音量回落到底噪以上多少 dB 以内。要小于 enterMarginDb，形成回差。 */
  exitMarginDb: number;
  /** 进入阈值的下限（dBFS）。再安静的房间，低于这个音量也不算说话。 */
  minEnterDb: number;
  /** 超过进入阈值要持续多久才算开始说话（毫秒）。 */
  minSpeechMs: number;
  /** 低于退出阈值要持续多久才算停止说话（毫秒）。 */
  hangoverMs: number;
  /** 估计底噪时回看多久（毫秒）。持续这么久没有间隙的声音会被当成背景噪声。 */
  floorWindowMs: number;
}

export const DEFAULT_ENERGY_VAD_CONFIG: EnergyVadConfig = {
  enterMarginDb: 12,
  exitMarginDb: 6,
  minEnterDb: -50,
  minSpeechMs: 150,
  hangoverMs: 400,
  floorWindowMs: 3000,
};

export type VadEvent = { type: "speech_start" } | { type: "speech_end" };

export interface VadSnapshot {
  speaking: boolean;
  /** 当前帧音量（dBFS）。 */
  levelDb: number;
  /** 当前底噪估计（dBFS）。 */
  floorDb: number;
}

/** 静音时 RMS 为 0，取对数会得到负无穷，这里给个下限。 */
const SILENCE_DB = -100;
/** 底噪估计的下限。采集偶尔会出现整帧为 0 的数字静音，不能让它把底噪拉到负一百。 */
const FLOOR_MIN_DB = -80;
/** 底噪按桶记最小值，桶的时长（毫秒）。 */
const FLOOR_BUCKET_MS = 250;

export function rmsToDb(rms: number): number {
  if (!(rms > 0)) return SILENCE_DB;
  return Math.max(SILENCE_DB, 20 * Math.log10(rms));
}

export class EnergyVad {
  private readonly config: EnergyVadConfig;
  /** 最近若干个桶各自的最低音量，最旧的在前。 */
  private readonly floorBuckets: number[] = [];
  private bucketMinDb = Number.POSITIVE_INFINITY;
  private bucketElapsedMs = 0;
  private floorDb = FLOOR_MIN_DB;
  private speaking = false;
  private aboveMs = 0;
  private belowMs = 0;
  private levelDb = SILENCE_DB;

  constructor(config: Partial<EnergyVadConfig> = {}) {
    this.config = { ...DEFAULT_ENERGY_VAD_CONFIG, ...config };
  }

  /**
   * 送入一帧。
   * @param rms 这一帧的均方根音量，线性值，0 到 1。
   * @param frameMs 这一帧的时长（毫秒）。
   */
  process(rms: number, frameMs: number): VadEvent | null {
    const level = rmsToDb(rms);
    this.levelDb = level;
    const { config } = this;

    this.updateFloor(level, frameMs);

    const enterDb = Math.max(this.floorDb + config.enterMarginDb, config.minEnterDb);
    const exitDb = Math.max(
      this.floorDb + config.exitMarginDb,
      config.minEnterDb - (config.enterMarginDb - config.exitMarginDb),
    );

    if (!this.speaking) {
      if (level >= enterDb) {
        this.aboveMs += frameMs;
        if (this.aboveMs >= config.minSpeechMs) {
          this.speaking = true;
          this.aboveMs = 0;
          this.belowMs = 0;
          return { type: "speech_start" };
        }
      } else {
        this.aboveMs = 0;
      }
      return null;
    }

    if (level < exitDb) {
      this.belowMs += frameMs;
      if (this.belowMs >= config.hangoverMs) {
        this.speaking = false;
        this.aboveMs = 0;
        this.belowMs = 0;
        return { type: "speech_end" };
      }
    } else {
      this.belowMs = 0;
    }
    return null;
  }

  private updateFloor(levelDb: number, frameMs: number): void {
    this.bucketMinDb = Math.min(this.bucketMinDb, Math.max(levelDb, FLOOR_MIN_DB));
    this.bucketElapsedMs += frameMs;
    if (this.bucketElapsedMs >= FLOOR_BUCKET_MS) {
      this.floorBuckets.push(this.bucketMinDb);
      const maxBuckets = Math.max(1, Math.round(this.config.floorWindowMs / FLOOR_BUCKET_MS));
      while (this.floorBuckets.length > maxBuckets) this.floorBuckets.shift();
      this.bucketMinDb = Number.POSITIVE_INFINITY;
      this.bucketElapsedMs = 0;
    }
    this.floorDb = Math.min(this.bucketMinDb, ...this.floorBuckets);
  }

  /**
   * 回到「没在说话」的状态，保留已经学到的底噪。
   * 每道题开始听作答时调用：面试官语音刚播完，不能把之前的状态带进来。
   */
  reset(): void {
    this.speaking = false;
    this.aboveMs = 0;
    this.belowMs = 0;
  }

  snapshot(): VadSnapshot {
    return { speaking: this.speaking, levelDb: this.levelDb, floorDb: this.floorDb };
  }
}
