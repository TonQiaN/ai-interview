import { clsx } from "clsx";
import type { CSSProperties, HTMLAttributes, ReactNode, TableHTMLAttributes } from "react";

export type Tone = "neutral" | "success" | "warning" | "danger" | "info";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: Tone;
  /** 不要前面那一点颜料 */
  plain?: boolean;
}

export function Badge({ tone = "neutral", plain, className, ...rest }: BadgeProps) {
  return (
    <span
      className={clsx("oil-badge", tone !== "neutral" && `oil-badge--${tone}`, plain && "oil-badge--plain", className)}
      {...rest}
    />
  );
}

export function Chip({ className, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return <span className={clsx("oil-chip", className)} {...rest} />;
}

export interface TableProps extends TableHTMLAttributes<HTMLTableElement> {
  hover?: boolean;
}

/** 表格外面自带横向滚动容器，窄屏不会撑破页面。 */
export function Table({ hover = true, className, ...rest }: TableProps) {
  return (
    <div className="oil-table-wrap">
      <table className={clsx("oil-table", hover && "oil-table--hover", className)} {...rest} />
    </div>
  );
}

export interface StatProps {
  label: ReactNode;
  value: ReactNode;
  unit?: ReactNode;
  delta?: ReactNode;
  className?: string;
}

export function Stat({ label, value, unit, delta, className }: StatProps) {
  return (
    <div className={clsx("oil-stat", className)}>
      <div className="oil-stat__label">{label}</div>
      <div className="oil-stat__value">
        {value}
        {unit && <small>{unit}</small>}
      </div>
      {delta && <div className="oil-stat__delta">{delta}</div>}
    </div>
  );
}

/** 由名字稳定地映射到 0..6 的一个颜料色，同一个人每次都是同一个颜色。 */
export function toneOf(name: string): number {
  let h = 0;
  for (const ch of name) h = (h * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  return h % 7;
}

export interface AvatarProps extends HTMLAttributes<HTMLSpanElement> {
  name: string;
  size?: "md" | "lg" | "xl";
}

/** 头像：一团颜料上写名字的第一个字。 */
export function Avatar({ name, size = "md", className, ...rest }: AvatarProps) {
  const first = Array.from(name.trim())[0] ?? "?";
  return (
    <span
      className={clsx("oil-avatar", size !== "md" && `oil-avatar--${size}`, className)}
      data-tone={toneOf(name)}
      aria-hidden="true"
      {...rest}
    >
      {first.toUpperCase()}
    </span>
  );
}

export interface KeyValueProps {
  items: ReadonlyArray<readonly [ReactNode, ReactNode]>;
  className?: string;
}

export function KeyValue({ items, className }: KeyValueProps) {
  return (
    <dl className={clsx("oil-kv", className)}>
      {items.map(([k, v], i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: 静态的键值列表，顺序不变
        <div key={i} style={{ display: "contents" }}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

export interface ScoreBarProps {
  label: ReactNode;
  /** 0 到 max */
  value: number;
  max?: number;
  /** 颜色，默认按分数高低取 */
  color?: string;
  className?: string;
}

function scoreColor(ratio: number): string {
  if (ratio >= 0.75) return "var(--oil-success)";
  if (ratio >= 0.5) return "var(--oil-gold)";
  return "var(--oil-danger)";
}

/** 一项评分：标签、分数，和一笔按分数长短抹出来的颜料。 */
export function ScoreBar({ label, value, max = 100, color, className }: ScoreBarProps) {
  const ratio = Math.max(0, Math.min(1, value / max));
  return (
    <div className={clsx("oil-score", className)}>
      <span className="oil-score__label">{label}</span>
      <span className="oil-score__value">{value}</span>
      {/* biome-ignore lint/a11y/useSemanticElements: 原生 meter 没法用笔触蒙版画，这里用 div 加 role */}
      <div
        className="oil-score__track"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-label={typeof label === "string" ? label : undefined}
      >
        <div
          className="oil-score__stroke"
          style={{ "--_v": `${ratio * 100}%`, "--_c": color ?? scoreColor(ratio) } as CSSProperties}
        />
      </div>
    </div>
  );
}
