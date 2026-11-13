import { clsx } from "clsx";
import type { ButtonHTMLAttributes, ReactNode } from "react";

export type ButtonVariant = "primary" | "accent" | "gold" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** 撑满一行。手机上的主操作一般都撑满 */
  block?: boolean;
  /** 只有图标的方形按钮，必须同时给 aria-label */
  iconOnly?: boolean;
  loading?: boolean;
  icon?: ReactNode;
}

export function buttonClass(
  o: Pick<ButtonProps, "variant" | "size" | "block" | "iconOnly"> = {},
  extra?: string,
): string {
  const { variant = "primary", size = "md", block, iconOnly } = o;
  return clsx(
    "oil-btn",
    variant !== "primary" && `oil-btn--${variant}`,
    size !== "md" && `oil-btn--${size}`,
    block && "oil-btn--block",
    iconOnly && "oil-btn--icon",
    extra,
  );
}

export function Button({
  variant,
  size,
  block,
  iconOnly,
  loading,
  icon,
  className,
  children,
  disabled,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={buttonClass({ variant, size, block, iconOnly }, className)}
      disabled={disabled || loading}
      data-loading={loading || undefined}
      aria-busy={loading || undefined}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
}
