import { clsx } from "clsx";
import { X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { buttonClass } from "./Button";

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  /** 底部的按钮区 */
  footer?: ReactNode;
  wide?: boolean;
  /** 点遮罩不关闭（用在必须明确选择的确认框上） */
  modalOnly?: boolean;
  className?: string;
}

/** 基于原生 dialog：焦点圈定、Esc 关闭、遮罩都由浏览器负责。 */
export function Dialog({ open, onClose, title, children, footer, wide, modalOnly, className }: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open && !el.open) el.showModal();
    if (!open && el.open) el.close();
  }, [open]);

  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: 键盘用 Esc 关闭，由原生 dialog 的 cancel 事件处理
    <dialog
      ref={ref}
      className={clsx("oil-dialog", wide && "oil-dialog--wide", className)}
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        // 点在 dialog 自身上就是点在遮罩上：内容区都在子元素里
        if (!modalOnly && e.target === ref.current) onClose();
      }}
    >
      <div className="oil-dialog__head">
        <div className="oil-dialog__title" id={titleId}>
          {title}
        </div>
        <button
          type="button"
          className={buttonClass({ variant: "ghost", size: "sm", iconOnly: true })}
          aria-label="关闭"
          onClick={onClose}
        >
          <X aria-hidden="true" />
        </button>
      </div>
      <div className="oil-dialog__body">{children}</div>
      {footer && <div className="oil-dialog__foot">{footer}</div>}
    </dialog>
  );
}
