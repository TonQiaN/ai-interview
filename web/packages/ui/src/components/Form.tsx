import { clsx } from "clsx";
import { Copy, Check as CheckIcon, UploadCloud } from "lucide-react";
import {
  type ChangeEvent,
  type CSSProperties,
  type DragEvent,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
  useId,
  useRef,
  useState,
} from "react";

export interface FieldProps {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  /** 关联到里面那个控件的 id；不给就自动生成并通过 render prop 传下去 */
  htmlFor?: string;
  className?: string;
  children: ReactNode | ((ids: { id: string; describedBy?: string }) => ReactNode);
}

/** 标签 + 控件 + 说明或报错。children 可以是函数，拿到自动生成的 id。 */
export function Field({ label, hint, error, required, htmlFor, className, children }: FieldProps) {
  const auto = useId();
  const id = htmlFor ?? auto;
  const noteId = hint || error ? `${id}-note` : undefined;
  return (
    <div className={clsx("oil-field", className)}>
      <label className="oil-field__label" htmlFor={id}>
        {label}
        {required && (
          <span className="oil-field__req" aria-hidden="true">
            *
          </span>
        )}
      </label>
      {typeof children === "function" ? children({ id, describedBy: noteId }) : children}
      {error ? (
        <div className="oil-field__error" id={noteId} role="alert">
          {error}
        </div>
      ) : hint ? (
        <div className="oil-field__hint" id={noteId}>
          {hint}
        </div>
      ) : null}
    </div>
  );
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={clsx("oil-input", className)} {...rest} />;
}

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={clsx("oil-textarea", className)} {...rest} />;
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={clsx("oil-select", className)} {...rest}>
      {children}
    </select>
  );
}

export interface CheckProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  label: ReactNode;
}

export function Checkbox({ label, className, ...rest }: CheckProps) {
  return (
    <label className={clsx("oil-check", className)}>
      <input type="checkbox" {...rest} />
      <span>{label}</span>
    </label>
  );
}

export function Radio({ label, className, ...rest }: CheckProps) {
  return (
    <label className={clsx("oil-check", className)}>
      <input type="radio" {...rest} />
      <span>{label}</span>
    </label>
  );
}

export interface SwitchProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  label?: ReactNode;
}

export function Switch({ label, className, ...rest }: SwitchProps) {
  return (
    <label className={clsx("oil-switch", className)}>
      {/* biome-ignore lint/a11y/useAriaPropsForRole: 原生复选框的 checked 就是开关状态，不需要再写 aria-checked */}
      <input type="checkbox" role="switch" {...rest} />
      {label && <span>{label}</span>}
    </label>
  );
}

export interface SliderProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "value" | "onChange"> {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  /** 右边显示的当前值，比如 (v) => `${v} 秒` */
  format?: (value: number) => ReactNode;
}

export function Slider({ value, min, max, step = 1, onChange, format, className, ...rest }: SliderProps) {
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;
  const input = (
    <input
      type="range"
      className={clsx("oil-slider", className)}
      style={{ "--_pct": `${pct}%` } as CSSProperties}
      value={value}
      min={min}
      max={max}
      step={step}
      onChange={(e: ChangeEvent<HTMLInputElement>) => onChange(Number(e.target.value))}
      {...rest}
    />
  );
  if (!format) return input;
  return (
    <div className="oil-slider-row">
      {input}
      <output>{format(value)}</output>
    </div>
  );
}

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
}

export interface SegmentedProps<T extends string> {
  value: T;
  options: ReadonlyArray<SegmentedOption<T>>;
  onChange: (value: T) => void;
  "aria-label": string;
  className?: string;
}

/** 几个互斥的选项排成一行，比如思考强度 低 / 高 / 最高。 */
export function Segmented<T extends string>({ value, options, onChange, className, ...rest }: SegmentedProps<T>) {
  return (
    <div className={clsx("oil-segmented", className)} role="radiogroup" aria-label={rest["aria-label"]}>
      {options.map((o) => (
        // biome-ignore lint/a11y/useSemanticElements: 分段选择用按钮实现单选组，键盘和读屏都按 radio 语义走
        <button key={o.value} type="button" role="radio" aria-checked={o.value === value} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export interface DropzoneProps {
  onFiles: (files: File[]) => void;
  accept?: string;
  multiple?: boolean;
  title?: ReactNode;
  hint?: ReactNode;
  icon?: ReactNode;
  className?: string;
}

/** 点一下选文件，或者把文件拖进来。 */
export function Dropzone({ onFiles, accept, multiple, title, hint, icon, className }: DropzoneProps) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  const take = (list: FileList | null) => {
    if (list?.length) onFiles(Array.from(list));
  };
  return (
    <button
      type="button"
      className={clsx("oil-dropzone", className)}
      data-over={over || undefined}
      onClick={() => input.current?.click()}
      onDragOver={(e: DragEvent) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e: DragEvent) => {
        e.preventDefault();
        setOver(false);
        take(e.dataTransfer.files);
      }}
    >
      {icon ?? <UploadCloud aria-hidden="true" />}
      <strong>{title ?? "点击选择文件，或拖到这里"}</strong>
      {hint && <span className="oil-field__hint">{hint}</span>}
      <input
        ref={input}
        type="file"
        hidden
        accept={accept}
        multiple={multiple}
        onChange={(e) => {
          take(e.target.files);
          e.target.value = "";
        }}
      />
    </button>
  );
}

export interface CopyFieldProps {
  value: string;
  label?: string;
  onCopied?: () => void;
  className?: string;
}

/** 一行可复制的文字，比如面试链接。 */
export function CopyField({ value, label = "复制", onCopied, className }: CopyFieldProps) {
  const [done, setDone] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // 剪贴板不可用（非安全上下文或被拒绝）时，退回到选中文字让用户自己复制
      return;
    }
    setDone(true);
    onCopied?.();
    window.setTimeout(() => setDone(false), 1600);
  };
  return (
    <div className={clsx("oil-copy", className)}>
      <code title={value}>{value}</code>
      <button type="button" onClick={copy} aria-live="polite">
        {done ? <CheckIcon aria-hidden="true" /> : <Copy aria-hidden="true" />}
        {done ? "已复制" : label}
      </button>
    </div>
  );
}
