import { VideoFrame, type VideoFrameProps } from "@ai-interview/ui";
import { useEffect, useRef } from "react";
import { useText } from "../text";

export interface CameraPreviewProps extends Omit<VideoFrameProps, "children" | "mirror"> {
  stream: MediaStream | null;
}

/** 候选人自己的画面。镜像显示，和照镜子一样；录下来的不镜像。 */
export function CameraPreview({ stream, ...rest }: CameraPreviewProps) {
  const ref = useRef<HTMLVideoElement>(null);
  const t = useText();
  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    video.srcObject = stream;
    return () => {
      video.srcObject = null;
    };
  }, [stream]);

  return (
    <VideoFrame mirror {...rest}>
      {stream ? (
        <video ref={ref} autoPlay muted playsInline aria-label={t.cameraLabel} />
      ) : undefined}
    </VideoFrame>
  );
}
