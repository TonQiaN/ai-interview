import { LevelMeter } from "@ai-interview/ui";
import { useEffect, useState } from "react";

export interface MicMeterProps {
  /** 订阅麦克风音量（0 到 1）。 */
  subscribe: (listener: (level: number) => void) => () => void;
  bars?: number;
  className?: string;
}

/** 多久往右挪一格（毫秒）。音量每 20 毫秒来一次，没必要每次都重画。 */
const STEP_MS = 80;

/** 滚动的音量条：最右边是此刻的音量。 */
export function MicMeter({ subscribe, bars = 20, className }: MicMeterProps) {
  const [levels, setLevels] = useState<number[]>(() => new Array<number>(bars).fill(0));

  useEffect(() => {
    let peak = 0;
    const unsubscribe = subscribe((level) => {
      peak = Math.max(peak, level);
    });
    const timer = setInterval(() => {
      const next = peak;
      peak = 0;
      setLevels((previous) => [...previous.slice(1), next]);
    }, STEP_MS);
    return () => {
      unsubscribe();
      clearInterval(timer);
    };
  }, [subscribe]);

  return <LevelMeter levels={levels} className={className} />;
}
