import { useState } from "react";
import { clock } from "../lib/format";
import type { Frame } from "../lib/frames";

/** 一次最多摆几张。一段时间里可能存了很多帧，看前几张就知道是怎么回事 */
const LIMIT = 3;

function Thumb({ frame, onPick }: { frame: Frame; onPick?: (ms: number) => void }) {
  const [missing, setMissing] = useState(false);
  // 这一帧没存图（接口返回 404）就不占地方
  if (missing) return null;
  const image = (
    <img
      src={frame.url}
      alt={`录像 ${clock(frame.at_ms)} 的画面`}
      loading="lazy"
      onError={() => setMissing(true)}
    />
  );
  return onPick ? (
    <button
      type="button"
      className="adm-frame"
      title="跳到回看的这个位置"
      onClick={() => onPick(frame.at_ms)}
    >
      {image}
    </button>
  ) : (
    <span className="adm-frame">{image}</span>
  );
}

/** 录像里不正常的那几帧。给了 onPick 就能点，点了跳到回看的那个时间 */
export function FrameThumbs({
  frames,
  onPick,
}: {
  frames: Frame[] | undefined;
  onPick?: (ms: number) => void;
}) {
  if (!frames || frames.length === 0) return null;
  const more = frames.length - LIMIT;
  return (
    <span className="adm-frames">
      {frames.slice(0, LIMIT).map((frame) => (
        <Thumb key={frame.at_ms} frame={frame} onPick={onPick} />
      ))}
      {more > 0 && <span className="adm-frames__more">还有 {more} 帧</span>}
    </span>
  );
}
