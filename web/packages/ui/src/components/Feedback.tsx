import { clsx } from "clsx";
import { AlertTriangle, CheckCircle2, Info, XCircle } from "lucide-react";
import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import type { PaintingName } from "../paintings";
import type { Tone } from "./Data";
import { Painting } from "./Surface";

const bannerIcon = {
  info: Info,
  neutral: Info,
  success: CheckCircle2,
  warning: AlertTriangle,
  danger: XCircle,
} as const;

export interface BannerProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  tone?: Exclude<Tone, "neutral">;
  title?: ReactNode;
  /** 贴在页面最上面（断网、重连、上传进度） */
  top?: boolean;
  actions?: ReactNode;
}

export function Banner({ tone = "info", title, top, actions, className, children, ...rest }: BannerProps) {
  const Icon = bannerIcon[tone];
  return (
    <div
      className={clsx("oil-banner", tone !== "info" && `oil-banner--${tone}`, top && "oil-banner--top", className)}
      role={tone === "danger" ? "alert" : "status"}
      {...rest}
    >
      <Icon aria-hidden="true" />
      <div className="oil-banner__body">
        {title && <div className="oil-banner__title">{title}</div>}
        {children}
      </div>
      {actions}
    </div>
  );
}

export interface ProgressProps {
  /** 0 到 1 */
  value: number;
  tone?: "primary" | "success" | "warning" | "danger" | "accent" | "gold";
  small?: boolean;
  label?: string;
  className?: string;
}

export function Progress({ value, tone = "primary", small, label, className }: ProgressProps) {
  const v = Math.max(0, Math.min(1, value));
  return (
    <div
      className={clsx("oil-progress", tone !== "primary" && `oil-progress--${tone}`, small && "oil-progress--sm", className)}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(v * 100)}
      aria-label={label}
    >
      <div className="oil-progress__bar" style={{ "--_v": v } as CSSProperties} />
    </div>
  );
}

export interface RingProps {
  /** 0 到 1 */
  value: number;
  size?: number;
  thickness?: number;
  color?: string;
  label?: string;
  children?: ReactNode;
  className?: string;
}

/** 环形进度，中间可以放数字。 */
export function Ring({ value, size = 96, thickness = 8, color, label, children, className }: RingProps) {
  const r = (size - thickness) / 2;
  const c = 2 * Math.PI * r;
  const v = Math.max(0, Math.min(1, value));
  return (
    <div
      className={clsx("oil-ring", className)}
      style={{ width: size, height: size, "--_c": color } as CSSProperties}
      role="img"
      aria-label={label ?? `${Math.round(v * 100)}%`}
    >
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
        <circle className="oil-ring__track" cx={size / 2} cy={size / 2} r={r} strokeWidth={thickness} />
        <circle
          className="oil-ring__value"
          cx={size / 2}
          cy={size / 2}
          r={r}
          strokeWidth={thickness}
          strokeDasharray={c}
          strokeDashoffset={c * (1 - v)}
        />
      </svg>
      {children != null && (
        <span className="oil-ring__label" style={{ fontSize: size * 0.3 }}>
          {children}
        </span>
      )}
    </div>
  );
}

export function Spinner({ label = "加载中" }: { label?: string }) {
  return <span className="oil-spinner" role="status" aria-label={label} />;
}

export function Skeleton({ width, height = 16, className }: { width?: number | string; height?: number | string; className?: string }) {
  return <div className={clsx("oil-skeleton", className)} style={{ width, height }} aria-hidden="true" />;
}

export interface EmptyStateProps {
  title: ReactNode;
  description?: ReactNode;
  painting?: PaintingName;
  actions?: ReactNode;
  className?: string;
}

export function EmptyState({ title, description, painting = "bouquet", actions, className }: EmptyStateProps) {
  return (
    <div className={clsx("oil-empty", className)}>
      <Painting name={painting} weight="thin" />
      <div className="oil-empty__title">{title}</div>
      {description && <div className="oil-empty__desc">{description}</div>}
      {actions}
    </div>
  );
}
