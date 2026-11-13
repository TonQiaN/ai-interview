/**
 * 申请摄像头和麦克风。
 */
import { classifyMediaError, type MediaErrorKind } from "./errors";

export interface CaptureOptions {
  /** 要不要摄像头。 */
  video: boolean;
  /** 期望的画面宽高和帧率。初筛面试不需要高清，低一些省电、省流量。 */
  width?: number;
  height?: number;
  frameRate?: number;
}

export class CaptureError extends Error {
  constructor(
    readonly kind: MediaErrorKind,
    cause: unknown,
  ) {
    super(`无法使用摄像头或麦克风：${kind}`, { cause });
    this.name = "CaptureError";
  }
}

export function captureConstraints(options: CaptureOptions): MediaStreamConstraints {
  return {
    audio: {
      // 回声消除：面试官的声音从扬声器出来，不能再被麦克风收进去触发静音检测
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
    },
    video: options.video
      ? {
          facingMode: "user",
          width: { ideal: options.width ?? 640 },
          height: { ideal: options.height ?? 480 },
          frameRate: { ideal: options.frameRate ?? 15, max: 30 },
        }
      : false,
  };
}

/**
 * @throws CaptureError 带上归好类的原因，页面据此给出对应的指引。
 */
export async function requestCapture(options: CaptureOptions): Promise<MediaStream> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new CaptureError(window.isSecureContext ? "unsupported" : "insecure_context", null);
  }
  try {
    return await navigator.mediaDevices.getUserMedia(captureConstraints(options));
  } catch (error) {
    throw new CaptureError(classifyMediaError(error), error);
  }
}

/** 停掉一条流的所有轨道，摄像头指示灯会熄灭。 */
export function stopStream(stream: MediaStream | null | undefined): void {
  for (const track of stream?.getTracks() ?? []) track.stop();
}
