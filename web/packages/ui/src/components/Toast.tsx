import { CheckCircle2 } from "lucide-react";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";

interface ToastItem {
  id: number;
  text: ReactNode;
}

const ToastContext = createContext<(text: ReactNode) => void>(() => {});

/** 包在应用最外层。里面任何地方用 useToast() 弹一条几秒后自动消失的提示。 */
export function ToastProvider({
  children,
  duration = 3200,
}: {
  children: ReactNode;
  duration?: number;
}) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const next = useRef(1);
  const push = useCallback(
    (text: ReactNode) => {
      const id = next.current++;
      setItems((list) => [...list, { id, text }]);
      window.setTimeout(() => setItems((list) => list.filter((t) => t.id !== id)), duration);
    },
    [duration],
  );
  const value = useMemo(() => push, [push]);
  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="oil-toasts" aria-live="polite">
        {items.map((t) => (
          <div className="oil-toast" key={t.id}>
            <CheckCircle2 aria-hidden="true" />
            <span>{t.text}</span>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): (text: ReactNode) => void {
  return useContext(ToastContext);
}
