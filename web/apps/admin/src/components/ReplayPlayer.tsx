/** 面试回看：录像（或没有录像时的时间轴）、可拖的进度条、带时间戳的逐字稿，点哪句跳到哪句。 */
import {
  Avatar,
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Select,
  VideoFrame,
} from "@ai-interview/ui";
import { Download, Pause, Play, RotateCcw, RotateCw } from "lucide-react";
import { type KeyboardEvent, type PointerEvent, useEffect, useMemo, useRef } from "react";
import type { InterviewEvent, Recording, Transcript } from "../api/types";
import { activeTurnIndex, useClock } from "../lib/clock";
import {
  clock,
  duration,
  eventLabel,
  eventNote,
  eventSeverity,
  eventText,
  fileSize,
  speakerLabel,
} from "../lib/format";

const rates = [0.75, 1, 1.25, 1.5, 2];
const trackLabel: Record<Recording["track"], string> = {
  camera: "录像",
  microphone: "录音",
  screen: "屏幕",
};

export interface ReplayPlayerProps {
  transcript: Transcript;
  recordings: Recording[];
  events: InterviewEvent[];
  candidateName: string;
  /** 要跳到的时间点。nonce 变了就跳，这样重复点同一处也有效 */
  seekTo?: { ms: number; nonce: number } | null;
}

export function ReplayPlayer({
  transcript,
  recordings,
  events,
  candidateName,
  seekTo,
}: ReplayPlayerProps) {
  const { turns } = transcript;
  const total = Math.max(transcript.duration_ms, turns.at(-1)?.end_ms ?? 0, 1);
  const video = recordings.find((r) => r.track === "camera" && r.media_url);
  const audio = video ? undefined : recordings.find((r) => r.track === "microphone" && r.media_url);
  const source = video ?? audio;
  const mediaRef = useRef<HTMLMediaElement | null>(null);
  const { ms, playing, rate, toggle, seek, setRate } = useClock(
    total,
    mediaRef,
    source?.start_ms ?? 0,
  );

  const active = activeTurnIndex(turns, ms);
  const current = turns[active];
  const speaking = current && ms <= current.end_ms ? current : undefined;
  const who = (speaker: string) =>
    speaker === "candidate" ? candidateName : (speakerLabel[speaker] ?? speaker);
  const marks = useMemo(
    () => events.filter((e) => e.at_ms !== null && eventSeverity(e.kind) !== "info"),
    [events],
  );

  useEffect(() => {
    if (seekTo) seek(seekTo.ms);
  }, [seekTo, seek]);

  // 播放时让正在说的那句留在视野里
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (active < 0) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-turn="${active}"]`);
    const box = listRef.current;
    if (!el || !box) return;
    const top = el.offsetTop - box.offsetTop;
    if (top < box.scrollTop || top + el.offsetHeight > box.scrollTop + box.clientHeight)
      box.scrollTop = Math.max(0, top - box.clientHeight / 3);
  }, [active]);

  const trackRef = useRef<HTMLDivElement | null>(null);
  function seekFromPointer(e: PointerEvent<HTMLDivElement>) {
    const box = trackRef.current?.getBoundingClientRect();
    if (!box || box.width === 0) return;
    seek(((e.clientX - box.left) / box.width) * total);
  }
  function onTrackKey(e: KeyboardEvent<HTMLDivElement>) {
    const step = e.shiftKey ? 30_000 : 5_000;
    if (e.key === "ArrowRight") seek(ms + step);
    else if (e.key === "ArrowLeft") seek(ms - step);
    else if (e.key === "Home") seek(0);
    else if (e.key === "End") seek(total);
    else if (e.key === " " || e.key === "Enter") toggle();
    else return;
    e.preventDefault();
  }
  const pct = (value: number) => `${(Math.min(total, Math.max(0, value)) / total) * 100}%`;

  return (
    <div className="adm-replay">
      <div className="adm-replay__stage">
        <VideoFrame
          placeholder={
            <div
              className="oil-stack"
              style={{ alignItems: "center", textAlign: "center", padding: "var(--oil-space-5)" }}
            >
              <Avatar name={candidateName} size="xl" />
              <div>{audio ? "这场面试只录了声音" : "这场面试没有可播放的录像"}</div>
              {!source && (
                <div style={{ opacity: 0.75, fontSize: "var(--oil-text-sm)" }}>
                  可以按时间轴回放逐字稿
                </div>
              )}
            </div>
          }
          topLeft={<Badge>{clock(ms)}</Badge>}
          topRight={
            speaking && (
              <Badge
                tone={speaking.speaker === "interviewer" ? "warning" : "info"}
              >{`${who(speaking.speaker)}在说`}</Badge>
            )
          }
        >
          {video ? (
            // biome-ignore lint/a11y/useMediaCaption: 逐字稿就在旁边，和画面同步滚动
            <video
              ref={mediaRef as React.RefObject<HTMLVideoElement>}
              src={video.media_url ?? undefined}
              playsInline
              preload="metadata"
            />
          ) : undefined}
        </VideoFrame>
        {audio && (
          // biome-ignore lint/a11y/useMediaCaption: 逐字稿就在旁边，和声音同步滚动
          <audio
            ref={mediaRef as React.RefObject<HTMLAudioElement>}
            src={audio.media_url ?? undefined}
            preload="metadata"
          />
        )}

        <div className="adm-replay__controls">
          <Button iconOnly aria-label={playing ? "暂停" : "播放"} onClick={toggle}>
            {playing ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}
          </Button>
          <Button
            variant="ghost"
            iconOnly
            aria-label="后退 10 秒"
            onClick={() => seek(ms - 10_000)}
          >
            <RotateCcw aria-hidden="true" />
          </Button>
          <Button
            variant="ghost"
            iconOnly
            aria-label="前进 10 秒"
            onClick={() => seek(ms + 10_000)}
          >
            <RotateCw aria-hidden="true" />
          </Button>
          <span className="adm-replay__time">
            {clock(ms)} / {clock(total)}
          </span>
          <span className="oil-grow" />
          <Select
            aria-label="播放速度"
            value={String(rate)}
            onChange={(e) => setRate(Number(e.target.value))}
            style={{ width: 96 }}
          >
            {rates.map((r) => (
              <option key={r} value={r}>
                {r} 倍
              </option>
            ))}
          </Select>
        </div>

        <div
          ref={trackRef}
          className="adm-track"
          role="slider"
          tabIndex={0}
          aria-label="播放进度"
          aria-valuemin={0}
          aria-valuemax={Math.round(total / 1000)}
          aria-valuenow={Math.round(ms / 1000)}
          aria-valuetext={`${clock(ms)}，共 ${clock(total)}`}
          onKeyDown={onTrackKey}
          onPointerDown={(e) => {
            if ((e.target as HTMLElement).closest("button")) return;
            e.currentTarget.setPointerCapture(e.pointerId);
            seekFromPointer(e);
          }}
          onPointerMove={(e) => {
            if (e.currentTarget.hasPointerCapture(e.pointerId)) seekFromPointer(e);
          }}
        >
          <div className="adm-track__rail">
            {turns.map((t) => (
              <span
                key={t.id}
                className={`adm-track__seg${t.speaker === "interviewer" ? " adm-track__seg--interviewer" : ""}`}
                style={{
                  left: pct(t.start_ms),
                  width: `calc(${pct(t.end_ms)} - ${pct(t.start_ms)})`,
                }}
              />
            ))}
          </div>
          <div className="adm-track__played" style={{ width: pct(ms) }} />
          <div className="adm-track__head" style={{ left: pct(ms) }} />
          {marks.map((e) => (
            <button
              key={e.id}
              type="button"
              className={`adm-track__mark${eventSeverity(e.kind) === "notice" ? " adm-track__mark--info" : ""}`}
              style={{ left: pct(e.at_ms ?? 0) }}
              title={`${clock(e.at_ms ?? 0)} ${eventText(e)}`}
              aria-label={`跳到 ${clock(e.at_ms ?? 0)}：${eventLabel[e.kind] ?? e.kind}`}
              onClick={() => seek((e.at_ms ?? 0) - 3000)}
            />
          ))}
        </div>
        <div className="adm-track__legend">
          <span>
            <i style={{ background: "var(--oil-pigment-ochre)" }} />
            面试官
          </span>
          <span>
            <i style={{ background: "var(--oil-pigment-cerulean)" }} />
            候选人
          </span>
          {marks.length > 0 && (
            <span>
              <i style={{ background: "var(--oil-warning)", borderRadius: "50%" }} />
              值得留意的时刻
            </span>
          )}
        </div>

        {(events.length > 0 || recordings.length > 0) && (
          <Card className="adm-section" flat>
            <CardHeader title="过程记录" />
            <CardBody>
              <ul className="adm-timeline">
                {events.map((e) => (
                  <li key={e.id}>
                    <span className="adm-turn__time">
                      {e.at_ms === null ? "开始前" : clock(e.at_ms)}
                    </span>
                    <span>
                      {e.at_ms === null ? (
                        (eventLabel[e.kind] ?? e.kind)
                      ) : (
                        <button
                          type="button"
                          className="adm-chip-btn"
                          onClick={() => seek((e.at_ms ?? 0) - 3000)}
                        >
                          {eventLabel[e.kind] ?? e.kind}
                        </button>
                      )}
                      {eventNote(e) && <span className="adm-item__sub"> {eventNote(e)}</span>}
                    </span>
                    {eventSeverity(e.kind) !== "info" && (
                      <Badge tone={eventSeverity(e.kind) === "warning" ? "warning" : "info"} plain>
                        {eventSeverity(e.kind) === "warning" ? "留意" : "提示"}
                      </Badge>
                    )}
                  </li>
                ))}
              </ul>
              {recordings.length > 0 && (
                <div className="oil-row oil-row--wrap" style={{ marginTop: "var(--oil-space-3)" }}>
                  {recordings.map((r) =>
                    r.media_url ? (
                      <a key={r.id} className="adm-chip-btn" href={r.media_url} download>
                        <Download
                          aria-hidden="true"
                          size={13}
                          style={{ verticalAlign: -2, marginRight: 4 }}
                        />
                        {trackLabel[r.track]} {duration(r.duration_ms)} · {fileSize(r.size_bytes)}
                      </a>
                    ) : (
                      <span key={r.id} className="adm-item__sub">
                        {trackLabel[r.track]} {duration(r.duration_ms)} · {fileSize(r.size_bytes)}
                      </span>
                    ),
                  )}
                </div>
              )}
            </CardBody>
          </Card>
        )}
      </div>

      <Card>
        <CardHeader
          title="逐字稿"
          actions={<span className="adm-item__sub">点一句话跳到那里</span>}
        />
        <CardBody>
          {turns.length === 0 ? (
            <p className="oil-muted">还没有逐字稿。</p>
          ) : (
            <div className="adm-turns" ref={listRef}>
              {turns.map((t, index) => (
                <button
                  key={t.id}
                  type="button"
                  data-turn={index}
                  className={`adm-turn adm-turn--${t.speaker}`}
                  aria-current={index === active ? "true" : undefined}
                  onClick={() => seek(t.start_ms)}
                >
                  <span className="adm-turn__time">{clock(t.start_ms)}</span>
                  <span>
                    <span className="adm-turn__who">
                      {who(t.speaker)}
                      {t.transcript_source === "live" && " · 实时转写，还没校对"}
                    </span>
                    <p className="adm-turn__text">{t.text}</p>
                  </span>
                </button>
              ))}
            </div>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
