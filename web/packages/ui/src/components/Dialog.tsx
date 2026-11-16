import { clsx } from "clsx";
import { X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { useUiText } from "../text";
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
  /** 必须在弹窗里的按钮中选一个才能离开：没有关闭按钮，按 Esc、点遮罩都不关 */
  persistent?: boolean;
  /**
   * 里面有填了还没保存的内容：按 Esc、点遮罩不关（很容易误触），并提示一句内容还没保存。
   * 右上角的关闭按钮和底部的按钮照常能关。
   */
  unsaved?: boolean;
  className?: string;
}

/** 基于原生 dialog：焦点圈定、Esc 关闭、遮罩都由浏览器负责。 */
export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  wide,
  modalOnly,
  persistent,
  unsaved,
  className,
}: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const text = useUiText();
  // 因为内容没保存而没关成：提示一句，不然按了 Esc 没反应会以为卡住了
  const [held, setHeld] = useState(false);
  if (held && (!open || !unsaved)) setHeld(false);

  /** 按 Esc、点遮罩：不是明确要关，有没保存的内容就留着 */
  function dismiss() {
    if (unsaved) setHeld(true);
    else onClose();
  }

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
        if (!persistent) dismiss();
      }}
      onClick={(e) => {
        // 点在 dialog 自身上就是点在遮罩上：内容区都在子元素里
        if (!modalOnly && !persistent && e.target === ref.current) dismiss();
      }}
    >
      <div className="oil-dialog__head">
        <div className="oil-dialog__title" id={titleId}>
          {title}
        </div>
        {!persistent && (
          <button
            type="button"
            className={buttonClass({ variant: "ghost", size: "sm", iconOnly: true })}
            aria-label={text.close}
            onClick={onClose}
          >
            <X aria-hidden="true" />
          </button>
        )}
      </div>
      <div className="oil-dialog__body">{children}</div>
      {held && (
        <p className="oil-dialog__hint" role="status">
          {text.unsavedHint}
        </p>
      )}
      {footer && <div className="oil-dialog__foot">{footer}</div>}
    </dialog>
  );
}
