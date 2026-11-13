/**
 * 回看用的时钟。有录像文件时跟着 <video> 走；没有的时候自己走表，
 * 这样只有逐字稿也能按时间轴回放。
 */
import { type RefObject, useCallback, useEffect, useRef, useState } from "react";

export interface Clock {
  ms: number;
  playing: boolean;
  rate: number;
  toggle: () => void;
  seek: (ms: number) => void;
  setRate: (rate: number) => void;
}

/**
 * @param durationMs 面试总长
 * @param media 录像或录音的元素；没有文件时它的 current 是 null
 * @param mediaStartMs 这段录制在面试时间轴上从哪里开始
 */
export function useClock(
  durationMs: number,
  media: RefObject<HTMLMediaElement | null>,
  mediaStartMs = 0,
): Clock {
  const [ms, setMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [rate, setRateState] = useState(1);
  const msRef = useRef(0);
  msRef.current = ms;

  // 没有媒体文件：每 100 毫秒按真实流逝的时间往前走
  useEffect(() => {
    if (!playing || media.current) return;
    let last = performance.now();
    const timer = setInterval(() => {
      const now = performance.now();
      const next = Math.min(durationMs, msRef.current + (now - last) * rate);
      last = now;
      setMs(next);
      if (next >= durationMs) setPlaying(false);
    }, 100);
    return () => clearInterval(timer);
  }, [playing, rate, durationMs, media]);

  // 有媒体文件：听它的事件
  useEffect(() => {
    const el = media.current;
    if (!el) return;
    const onTime = () => setMs(Math.min(durationMs, el.currentTime * 1000 + mediaStartMs));
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    el.addEventListener("timeupdate", onTime);
    el.addEventListener("play", onPlay);
    el.addEventListener("pause", onPause);
    el.addEventListener("ended", onPause);
    return () => {
      el.removeEventListener("timeupdate", onTime);
      el.removeEventListener("play", onPlay);
      el.removeEventListener("pause", onPause);
      el.removeEventListener("ended", onPause);
    };
  }, [media, durationMs, mediaStartMs]);

  const seek = useCallback(
    (target: number) => {
      const next = Math.min(durationMs, Math.max(0, target));
      setMs(next);
      const el = media.current;
      if (el) el.currentTime = Math.max(0, (next - mediaStartMs) / 1000);
    },
    [durationMs, media, mediaStartMs],
  );

  const toggle = useCallback(() => {
    const el = media.current;
    if (el) {
      if (el.paused) void el.play();
      else el.pause();
      return;
    }
    setPlaying((was) => {
      // 播到头了再点播放，从头来
      if (!was && msRef.current >= durationMs) setMs(0);
      return !was;
    });
  }, [media, durationMs]);

  const setRate = useCallback(
    (next: number) => {
      setRateState(next);
      if (media.current) media.current.playbackRate = next;
    },
    [media],
  );

  return { ms, playing, rate, toggle, seek, setRate };
}

/** 现在说到哪一句：最后一句已经开始的话。还没开始时是 -1。 */
export function activeTurnIndex(turns: ReadonlyArray<{ start_ms: number }>, ms: number): number {
  let lo = 0;
  let hi = turns.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((turns[mid]?.start_ms ?? Number.POSITIVE_INFINITY) <= ms) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}
