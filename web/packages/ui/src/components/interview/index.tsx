// 候选人面试页用的组件。逻辑（取流、检测、计时、上传）都由使用方做，这里只管长什么样。
import { clsx } from "clsx";
import { CheckCircle2, ChevronDown, Clock, XCircle } from "lucide-react";
import { type CSSProperties, type HTMLAttributes, type ReactNode, useId, useState } from "react";
import { type PaintingName, paintings } from "../../paintings";
import { Frame, type FrameProps, Painting } from "../Surface";

export interface StageProps extends HTMLAttributes<HTMLDivElement> {
  /** 深色舞台，面试进行中用 */
  night?: boolean;
  /** 放宽到 960px，横屏或桌面上左右分栏时用 */
  wide?: boolean;
  header?: ReactNode;
  footer?: ReactNode;
}

/** 候选人页的外壳：窄栏、满屏高、留出刘海和底部安全区。footer 会贴在底部。 */
export function Stage({ night, wide, header, footer, className, children, ...rest }: StageProps) {
  return (
    <div className={clsx("oil-stage", night && "oil-stage--night", className)} {...rest}>
      <div className={clsx("oil-stage__col", wide && "oil-stage__col--wide")}>
        {header && <div className="oil-stage__head">{header}</div>}
        {children}
        {footer && <div className="oil-stage__foot">{footer}</div>}
      </div>
    </div>
  );
}

export type DeviceCheckState = "idle" | "checking" | "passed" | "failed";

export interface DeviceCheckCardProps {
  state: DeviceCheckState;
  icon: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  /** 卡片下方的内容：预览、音量条、重试按钮等 */
  children?: ReactNode;
  className?: string;
}

const deviceStateLabel: Record<DeviceCheckState, string> = {
  idle: "待检测",
  checking: "检测中",
  passed: "通过",
  failed: "未通过",
};

export function DeviceCheckCard({
  state,
  icon,
  title,
  description,
  children,
  className,
}: DeviceCheckCardProps) {
  return (
    <section className={clsx("oil-device", className)} data-state={state}>
      <span className="oil-device__icon" aria-hidden="true">
        {icon}
      </span>
      <div className="oil-device__title">{title}</div>
      <span className="oil-device__state" role="status" aria-label={deviceStateLabel[state]}>
        {state === "checking" && <span className="oil-spinner" aria-hidden="true" />}
        {state === "passed" && <CheckCircle2 aria-hidden="true" />}
        {state === "failed" && <XCircle aria-hidden="true" />}
      </span>
      {description && <div className="oil-device__desc">{description}</div>}
      {children && <div className="oil-device__extra">{children}</div>}
    </section>
  );
}

export interface ConsentItem {
  id: string;
  label: ReactNode;
  /** 展开后看到的详细说明 */
  detail?: ReactNode;
}

export interface ConsentListProps {
  items: ReadonlyArray<ConsentItem>;
  /** 已勾选的条目 id */
  checked: ReadonlySet<string>;
  onChange: (next: Set<string>) => void;
  className?: string;
}

/** 同意事项清单。是否全部勾选用 allConsented() 判断，据此决定主按钮能不能点。 */
export function ConsentList({ items, checked, onChange, className }: ConsentListProps) {
  const [open, setOpen] = useState<string | null>(null);
  const base = useId();
  return (
    <ul className={clsx("oil-consent", className)}>
      {items.map((item) => {
        const detailId = `${base}-${item.id}`;
        return (
          <li className="oil-consent__item" key={item.id}>
            <div className="oil-consent__row">
              <label className="oil-check">
                <input
                  type="checkbox"
                  checked={checked.has(item.id)}
                  onChange={(e) => {
                    const next = new Set(checked);
                    if (e.target.checked) next.add(item.id);
                    else next.delete(item.id);
                    onChange(next);
                  }}
                />
                <span>{item.label}</span>
              </label>
              {item.detail && (
                <button
                  type="button"
                  className="oil-consent__more"
                  aria-expanded={open === item.id}
                  aria-controls={detailId}
                  aria-label="查看详情"
                  onClick={() => setOpen(open === item.id ? null : item.id)}
                >
                  <ChevronDown aria-hidden="true" />
                </button>
              )}
            </div>
            {item.detail && open === item.id && (
              <div className="oil-consent__detail" id={detailId}>
                {item.detail}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

export function allConsented(
  items: ReadonlyArray<ConsentItem>,
  checked: ReadonlySet<string>,
): boolean {
  return items.every((i) => checked.has(i.id));
}

export interface VideoFrameProps extends Omit<FrameProps, "ratio"> {
  shape?: "landscape" | "portrait" | "wide";
  /** 自拍预览要镜像 */
  mirror?: boolean;
  /** 还没有画面时显示的文字 */
  placeholder?: ReactNode;
  topLeft?: ReactNode;
  topRight?: ReactNode;
  bottomLeft?: ReactNode;
  /** 传入 <video> 元素；流由使用方挂上去 */
  children?: ReactNode;
}

/** 画框样式的视频容器。 */
export function VideoFrame({
  shape = "landscape",
  mirror,
  placeholder,
  topLeft,
  topRight,
  bottomLeft,
  children,
  ...rest
}: VideoFrameProps) {
  return (
    <Frame {...rest}>
      <div
        className={clsx(
          "oil-video",
          shape === "portrait" && "oil-video--portrait",
          shape === "wide" && "oil-video--wide",
          mirror && "oil-video--mirror",
        )}
      >
        {children ?? <div className="oil-video__placeholder">{placeholder}</div>}
        {topLeft && <div className="oil-video__corner oil-video__corner--tl">{topLeft}</div>}
        {topRight && <div className="oil-video__corner oil-video__corner--tr">{topRight}</div>}
        {bottomLeft && <div className="oil-video__corner oil-video__corner--bl">{bottomLeft}</div>}
      </div>
    </Frame>
  );
}

export type PresenceState = "idle" | "speaking" | "listening" | "thinking";

const presenceLabel: Record<PresenceState, string> = {
  idle: "",
  speaking: "正在提问",
  listening: "正在听",
  thinking: "思考中",
};

export interface InterviewerPresenceProps {
  state: PresenceState;
  painting?: PaintingName;
  /** 覆盖默认的状态文字；传空字符串就不显示 */
  label?: string;
  /** 直径，比如 200 或 "40vw"。默认 min(46vw, 220px) */
  size?: number | string;
  className?: string;
}

/** 面试官形象：一幅圆形的油画肖像，外圈表达「正在提问 / 正在听 / 思考中」。 */
export function InterviewerPresence({
  state,
  painting = "sitter",
  label,
  size,
  className,
}: InterviewerPresenceProps) {
  const text = label ?? presenceLabel[state];
  const sized =
    size == null
      ? undefined
      : ({ "--_size": typeof size === "number" ? `${size}px` : size } as CSSProperties);
  return (
    <div className={clsx("oil-presence", className)} data-state={state} style={sized}>
      <span className="oil-presence__halo" />
      <span className="oil-presence__halo" />
      <span className="oil-presence__halo" />
      <div className="oil-presence__portrait">
        <img src={paintings[painting]} alt="AI 面试官" />
      </div>
      {text && (
        <span className="oil-presence__label" role="status">
          {text}
        </span>
      )}
    </div>
  );
}

export interface CaptionCardProps {
  /** 左上角，比如「第 3 题 / 共 10 题」 */
  meta?: ReactNode;
  /** 右上角，比如计时 */
  aside?: ReactNode;
  /** 用在浅色页面上 */
  paper?: boolean;
  children: ReactNode;
  className?: string;
}

/** 题目字幕卡。 */
export function CaptionCard({ meta, aside, paper, children, className }: CaptionCardProps) {
  return (
    <div className={clsx("oil-caption", paper && "oil-caption--paper", className)}>
      {(meta || aside) && (
        <div className="oil-caption__meta">
          <span>{meta}</span>
          <span>{aside}</span>
        </div>
      )}
      <div className="oil-caption__text">{children}</div>
    </div>
  );
}

export interface QuestionProgressProps {
  total: number;
  /** 当前是第几题，从 0 开始 */
  current: number;
  paper?: boolean;
  className?: string;
}

export function QuestionProgress({ total, current, paper, className }: QuestionProgressProps) {
  return (
    <div
      className={clsx("oil-qprogress", paper && "oil-qprogress--paper", className)}
      role="progressbar"
      aria-valuemin={1}
      aria-valuemax={total}
      aria-valuenow={Math.min(total, current + 1)}
      aria-label={`第 ${Math.min(total, current + 1)} 题，共 ${total} 题`}
    >
      {Array.from({ length: total }, (_, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: 固定长度的格子
        <span key={i} data-state={i < current ? "done" : i === current ? "current" : "todo"} />
      ))}
    </div>
  );
}

/** 把秒数写成 mm:ss，超过一小时写成 h:mm:ss。 */
export function formatClock(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`;
}

export function Timer({
  seconds,
  label = "已用时",
  className,
}: {
  seconds: number;
  label?: string;
  className?: string;
}) {
  return (
    <span className={clsx("oil-timer", className)} role="timer" aria-label={label}>
      <Clock aria-hidden="true" />
      {formatClock(seconds)}
    </span>
  );
}

export function RecordingIndicator({
  paper,
  children = "录制中",
}: {
  paper?: boolean;
  children?: ReactNode;
}) {
  return <span className={clsx("oil-rec", paper && "oil-rec--paper")}>{children}</span>;
}

export interface SilenceRingProps {
  /** 还剩多少秒 */
  remaining: number;
  /** 一共多少秒 */
  total: number;
  size?: number;
  className?: string;
}

/** 静音倒计时环：候选人停下来以后开始走，走完进入下一题。 */
export function SilenceRing({ remaining, total, size = 72, className }: SilenceRingProps) {
  const thickness = 5;
  const r = (size - thickness) / 2;
  const c = 2 * Math.PI * r;
  const ratio = total > 0 ? Math.max(0, Math.min(1, remaining / total)) : 0;
  return (
    <div
      className={clsx("oil-silence", className)}
      style={{ width: size, height: size }}
      role="timer"
      aria-label={`${Math.ceil(remaining)} 秒后进入下一题`}
    >
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        <circle
          className="oil-silence__track"
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={thickness}
        />
        <circle
          className="oil-silence__value"
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={thickness}
          strokeDasharray={c}
          strokeDashoffset={c * (1 - ratio)}
        />
      </svg>
      <span className="oil-silence__label">
        <span className="oil-silence__num">{Math.ceil(remaining)}</span>
        <span className="oil-silence__unit">秒</span>
      </span>
    </div>
  );
}

export interface LevelMeterProps {
  /** 每根条的高度，0 到 1。逻辑（取音量、平滑）由使用方做 */
  levels: ReadonlyArray<number>;
  className?: string;
}

/**
 * 音量条的 DOM 版本。要更高的刷新率就自己用 canvas 画，
 * 颜色用 token("--oil-meter-on") 等取。
 */
export function LevelMeter({ levels, className }: LevelMeterProps) {
  return (
    <div className={clsx("oil-meter", className)} aria-hidden="true">
      {levels.map((h, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: 固定数量的条
        <span key={i} data-off={h < 0.05 || undefined} style={{ "--_h": h } as CSSProperties} />
      ))}
    </div>
  );
}

export interface StatusPageProps {
  painting: PaintingName;
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
}

/** 整页状态：链接已过期、已完成、浏览器不支持、结束感谢。放在 Stage 里用。 */
export function StatusPage({ painting, title, description, actions, className }: StatusPageProps) {
  return (
    <div className={clsx("oil-status", className)}>
      <Painting name={painting} ratio="4 / 3" />
      <h1 className="oil-status__title">{title}</h1>
      {description && <p className="oil-status__desc">{description}</p>}
      {actions && <div className="oil-status__actions">{actions}</div>}
    </div>
  );
}
