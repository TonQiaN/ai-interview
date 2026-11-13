import { clsx } from "clsx";
import { Check } from "lucide-react";
import { Fragment, type ReactNode, useId } from "react";

export interface TabItem<T extends string> {
  value: T;
  label: ReactNode;
  count?: number;
}

export interface TabsProps<T extends string> {
  value: T;
  items: ReadonlyArray<TabItem<T>>;
  onChange: (value: T) => void;
  "aria-label": string;
  className?: string;
}

export function Tabs<T extends string>({ value, items, onChange, className, ...rest }: TabsProps<T>) {
  const base = useId();
  return (
    <div className={clsx("oil-tabs", className)} role="tablist" aria-label={rest["aria-label"]}>
      {items.map((item) => (
        <button
          key={item.value}
          id={`${base}-${item.value}`}
          type="button"
          role="tab"
          className="oil-tab"
          aria-selected={item.value === value}
          tabIndex={item.value === value ? 0 : -1}
          onClick={() => onChange(item.value)}
          onKeyDown={(e) => {
            if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
            const i = items.findIndex((x) => x.value === value);
            const next = items[(i + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length];
            if (next) {
              onChange(next.value);
              document.getElementById(`${base}-${next.value}`)?.focus();
            }
          }}
        >
          {item.label}
          {item.count != null && <span className="oil-tab__count">{item.count}</span>}
        </button>
      ))}
    </div>
  );
}

export interface StepsProps {
  steps: ReadonlyArray<ReactNode>;
  /** 当前是第几步，从 0 开始 */
  current: number;
  /** 只留圆点，不显示文字（手机上步骤多时用） */
  compact?: boolean;
  className?: string;
}

export function Steps({ steps, current, compact, className }: StepsProps) {
  return (
    <ol className={clsx("oil-steps", compact && "oil-steps--compact", className)}>
      {steps.map((label, i) => {
        const state = i < current ? "done" : i === current ? "current" : "todo";
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: 步骤是固定顺序的静态列表
          <Fragment key={i}>
            {i > 0 && <li className="oil-step__line" data-done={i <= current || undefined} aria-hidden="true" />}
            <li className="oil-step" data-state={state} aria-current={state === "current" ? "step" : undefined}>
              <span className="oil-step__dot">{state === "done" ? <Check aria-label="已完成" /> : i + 1}</span>
              <span className="oil-step__text">{label}</span>
            </li>
          </Fragment>
        );
      })}
    </ol>
  );
}
