/**
 * 盯着音频是不是还在运转。
 *
 * 手机上来电话、切到别的应用、锁屏，系统会把页面的音频整个停掉（AudioContext 变成
 * suspended 或 interrupted）；有的 iPhone 上状态还写着 running，时钟却不走了。
 * 音频一停，面试官的声音播不出来，麦克风的音量读不到，录像里也没有声音，
 * 面试不能照常往下走。这里负责发现它停了、试着让它重新转起来。
 */

/** 用到的 AudioContext 的那一小部分，测试里可以换成假的。 */
export interface WatchedContext {
  readonly state: string;
  readonly currentTime: number;
  resume(): Promise<void>;
  suspend(): Promise<void>;
  addEventListener(type: "statechange", listener: () => void): void;
  removeEventListener(type: "statechange", listener: () => void): void;
}

/**
 * 音频是怎么停的：`suspended`、`interrupted` 是浏览器报的状态，
 * `stalled` 是状态正常但时钟不走。
 */
export type OutputHalt = "suspended" | "interrupted" | "stalled";

export interface OutputWatchOptions {
  context: WatchedContext;
  /** 停了（带原因）或者恢复了（null）时调用，只在变化时调用。 */
  onChange: (halt: OutputHalt | null) => void;
  /** 多久看一次时钟（毫秒）。 */
  intervalMs?: number;
  /** 连着几次时钟没走才算停了。页面刚从后台回来时会有一两次没走，不能算。 */
  stallChecks?: number;
  /** 让音频重新运转时，每一步最多等多久（毫秒）。有的浏览器不让恢复时既不成功也不报错。 */
  stepTimeoutMs?: number;
  /** 恢复之后等多久看时钟走没走（毫秒）。 */
  confirmMs?: number;
}

const DEFAULTS = { intervalMs: 1_000, stallChecks: 3, stepTimeoutMs: 1_500, confirmMs: 250 };

export class OutputWatch {
  private readonly context: WatchedContext;
  private readonly onChange: (halt: OutputHalt | null) => void;
  private readonly intervalMs: number;
  private readonly stallChecks: number;
  private readonly stepTimeoutMs: number;
  private readonly confirmMs: number;

  private timer: ReturnType<typeof setInterval> | null = null;
  private lastTime: number;
  private stalledFor = 0;
  private current: OutputHalt | null = null;
  private reviving: Promise<boolean> | null = null;

  constructor(options: OutputWatchOptions) {
    this.context = options.context;
    this.onChange = options.onChange;
    this.intervalMs = options.intervalMs ?? DEFAULTS.intervalMs;
    this.stallChecks = options.stallChecks ?? DEFAULTS.stallChecks;
    this.stepTimeoutMs = options.stepTimeoutMs ?? DEFAULTS.stepTimeoutMs;
    this.confirmMs = options.confirmMs ?? DEFAULTS.confirmMs;
    this.lastTime = this.context.currentTime;
  }

  /** 现在是不是停着；没停是 null。 */
  get halt(): OutputHalt | null {
    return this.current;
  }

  start(): void {
    if (this.timer !== null) return;
    this.lastTime = this.context.currentTime;
    this.stalledFor = 0;
    this.context.addEventListener("statechange", this.onStateChange);
    this.timer = setInterval(this.check, this.intervalMs);
    this.onStateChange();
  }

  stop(): void {
    if (this.timer === null) return;
    clearInterval(this.timer);
    this.timer = null;
    this.context.removeEventListener("statechange", this.onStateChange);
  }

  /**
   * 试着让音频重新运转，返回有没有成功。
   * 有的浏览器只在用户点击的处理里才让恢复，所以页面上要留一个按钮来调它。
   */
  revive(): Promise<boolean> {
    this.reviving ??= this.doRevive().finally(() => {
      this.reviving = null;
    });
    return this.reviving;
  }

  private async doRevive(): Promise<boolean> {
    const { context } = this;
    if (context.state === "closed") return false;
    // 状态正常但时钟不走：先停再开才转得起来
    if (context.state === "running") await this.attempt(() => context.suspend());
    await this.attempt(() => context.resume());

    const before = context.currentTime;
    await new Promise((resolve) => setTimeout(resolve, this.confirmMs));
    const running = context.state === "running" && context.currentTime > before;
    this.lastTime = context.currentTime;
    if (running) {
      this.stalledFor = 0;
      this.set(null);
    } else {
      this.set(haltOf(context.state) ?? "stalled");
    }
    return running;
  }

  /** 不成功、不让、没反应，都当作这一步没做成，接着往下走。 */
  private async attempt(step: () => Promise<void>): Promise<void> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, this.stepTimeoutMs);
    });
    try {
      await Promise.race([step().catch(() => {}), timeout]);
    } catch {
      // step 本身同步抛错（很旧的浏览器）
    } finally {
      clearTimeout(timer);
    }
  }

  private onStateChange = (): void => {
    const halt = haltOf(this.context.state);
    // 变回 running 不马上算恢复：等时钟真的走了再说
    if (halt !== null) this.set(halt);
  };

  private check = (): void => {
    const { state, currentTime } = this.context;
    if (state === "closed") return;
    const advanced = currentTime > this.lastTime;
    this.lastTime = currentTime;

    const halt = haltOf(state);
    if (halt !== null) {
      this.stalledFor = 0;
      this.set(halt);
    } else if (advanced) {
      this.stalledFor = 0;
      this.set(null);
    } else {
      this.stalledFor += 1;
      if (this.stalledFor >= this.stallChecks) this.set("stalled");
    }
  };

  private set(halt: OutputHalt | null): void {
    // 已经停着的，原因变了不再报一次
    if ((halt === null) === (this.current === null)) return;
    this.current = halt;
    this.onChange(halt);
  }
}

function haltOf(state: string): OutputHalt | null {
  if (state === "running" || state === "closed") return null;
  return state === "interrupted" ? "interrupted" : "suspended";
}
