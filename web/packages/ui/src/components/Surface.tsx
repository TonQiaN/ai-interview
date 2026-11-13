import { clsx } from "clsx";
import type { CSSProperties, HTMLAttributes, ReactNode } from "react";
import { type PaintingName, paintings } from "../paintings";

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  flat?: boolean;
  interactive?: boolean;
}

export function Card({ flat, interactive, className, ...rest }: CardProps) {
  return (
    <div
      className={clsx("oil-card", flat && "oil-card--flat", interactive && "oil-card--interactive", className)}
      {...rest}
    />
  );
}

export function CardHeader({
  title,
  actions,
  className,
  children,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <div className={clsx("oil-card__head", className)}>
      {title && <div className="oil-card__title">{title}</div>}
      {children}
      {actions}
    </div>
  );
}

export function CardBody({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={clsx("oil-card__body", className)} {...rest} />;
}

export function CardFooter({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={clsx("oil-card__foot", className)} {...rest} />;
}

export interface FrameProps extends HTMLAttributes<HTMLDivElement> {
  /** 外框宽度 */
  weight?: "thin" | "normal" | "wide";
  /** 深色木框，不要鎏金 */
  dark?: boolean;
  /** 里面内容的宽高比，比如 "4 / 3" */
  ratio?: string;
}

/** 画框。里面放 img、video 或任何内容。 */
export function Frame({ weight = "normal", dark, ratio, className, children, ...rest }: FrameProps) {
  return (
    <div
      className={clsx("oil-frame", weight !== "normal" && `oil-frame--${weight}`, dark && "oil-frame--dark", className)}
      {...rest}
    >
      <div className="oil-frame__inner" style={ratio ? { aspectRatio: ratio } : undefined}>
        {children}
      </div>
    </div>
  );
}

export interface PaintingProps extends Omit<FrameProps, "children"> {
  name: PaintingName;
  /** 插画是装饰时留空；有含义时写一句描述 */
  alt?: string;
}

/** 装在画框里的一幅内置油画。 */
export function Painting({ name, alt = "", ratio = "1 / 1", ...rest }: PaintingProps) {
  return (
    <Frame ratio={ratio} {...rest}>
      <img src={paintings[name]} alt={alt} loading="lazy" decoding="async" />
    </Frame>
  );
}

export interface HeroProps extends HTMLAttributes<HTMLDivElement> {
  painting?: PaintingName;
}

/** 一幅画做底的横幅，文字压在左侧的暗部上。 */
export function Hero({ painting = "dusk", className, style, children, ...rest }: HeroProps) {
  return (
    <div
      className={clsx("oil-hero", className)}
      style={{ backgroundImage: `url(${paintings[painting]})`, ...style } as CSSProperties}
      {...rest}
    >
      <div className="oil-hero__body">{children}</div>
    </div>
  );
}
