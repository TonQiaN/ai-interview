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

/** 一段录制在面试时间轴上的位置。断线重连之后录像会分成几段 */
export interface MediaSegment {
  start_ms: number;
}

/** 这个时间点该放哪一段：最后一段已经开始的。都还没开始时是第一段 */
export function segmentAt(segments: ReadonlyArray<MediaSegment>, ms: number): number {
  let found = 0;
  for (let index = 0; index < segments.length; index += 1)
    if ((segments[index]?.start_ms ?? Number.POSITIVE_INFINITY) <= ms) found = index;
  return found;
}

/**
 * @param durationMs 面试总长
 * @param media 录像或录音的元素；没有文件时它的 current 是 null
 * @param segments 能播放的各段录制，按开始时间排好。元素的 src 由调用方按返回的 segment 来换
 */
export function useClock(
  durationMs: number,
  media: RefObject<HTMLMediaElement | null>,
  segments: ReadonlyArray<MediaSegment> = [],
): Clock & { segment: number } {
  const [ms, setMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [rate, setRateState] = useState(1);
  const [segment, setSegment] = useState(0);
  const msRef = useRef(0);
  msRef.current = ms;
  const segmentRef = useRef(0);
  segmentRef.current = segment;
  const segmentsRef = useRef(segments);
  segmentsRef.current = segments;
  const rateRef = useRef(1);
  rateRef.current = rate;
  // 换了一段之后，等新的一段加载好再跳到这个位置
  const pending = useRef<{ offsetSec: number; play: boolean } | null>(null);
  const hasMedia = segments.length > 0;

  // 没有媒体文件：每 100 毫秒按真实流逝的时间往前走
  useEffect(() => {
    if (!playing || hasMedia) return;
    let last = performance.now();
    const timer = setInterval(() => {
      const now = performance.now();
      const next = Math.min(durationMs, msRef.current + (now - last) * rate);
      last = now;
      setMs(next);
      if (next >= durationMs) setPlaying(false);
    }, 100);
    return () => clearInterval(timer);
  }, [playing, rate, durationMs, hasMedia]);

  // 有媒体文件：听它的事件
  useEffect(() => {
    const el = media.current;
    if (!el || !hasMedia) return;
    const startOf = (index: number) => segmentsRef.current[index]?.start_ms ?? 0;
    const onTime = () => {
      if (pending.current) return;
      setMs(Math.min(durationMs, el.currentTime * 1000 + startOf(segmentRef.current)));
    };
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onEnded = () => {
      const next = segmentRef.current + 1;
      if (next >= segmentsRef.current.length) return setPlaying(false);
      // 这一段放完了，接着放下一段
      pending.current = { offsetSec: 0, play: true };
      setSegment(next);
      setMs(Math.min(durationMs, startOf(next)));
    };
    const onLoaded = () => {
      const jump = pending.current;
      if (!jump) return;
      pending.current = null;
      el.currentTime = jump.offsetSec;
      el.playbackRate = rateRef.current;
      if (jump.play) void el.play();
    };
    el.addEventListener("timeupdate", onTime);
    el.addEventListener("play", onPlay);
    el.addEventListener("pause", onPause);
    el.addEventListener("ended", onEnded);
    el.addEventListener("loadedmetadata", onLoaded);
    return () => {
      el.removeEventListener("timeupdate", onTime);
      el.removeEventListener("play", onPlay);
      el.removeEventListener("pause", onPause);
      el.removeEventListener("ended", onEnded);
      el.removeEventListener("loadedmetadata", onLoaded);
    };
  }, [media, durationMs, hasMedia]);

  const seek = useCallback(
    (target: number) => {
      const next = Math.min(durationMs, Math.max(0, target));
      setMs(next);
      const el = media.current;
      const all = segmentsRef.current;
      if (!el || all.length === 0) return;
      const index = segmentAt(all, next);
      const offsetSec = Math.max(0, (next - (all[index]?.start_ms ?? 0)) / 1000);
      if (index === segmentRef.current && !pending.current) {
        el.currentTime = offsetSec;
        return;
      }
      pending.current = { offsetSec, play: pending.current?.play ?? !el.paused };
      setSegment(index);
    },
    [durationMs, media],
  );

  const toggle = useCallback(() => {
    const el = media.current;
    if (el && segmentsRef.current.length > 0) {
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

  return { ms, playing, rate, toggle, seek, setRate, segment };
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
