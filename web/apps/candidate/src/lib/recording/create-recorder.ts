/**
 * 用真实的 MediaRecorder 创建录制器，交给 SegmentRecorder。
 */
import { AUDIO_ONLY_MIME_CANDIDATES, pickRecordingMime, RECORDING_MIME_CANDIDATES } from "./mime";
import type { RecorderLike } from "./segment-recorder";

export interface RecorderSettings {
  /** 视频码率（比特每秒）。 */
  videoBitsPerSecond?: number;
  /** 音频码率（比特每秒）。 */
  audioBitsPerSecond?: number;
}

/** 默认码率：640×480 的人像画面够看清表情，30 分钟大约 120 MB。 */
export const DEFAULT_RECORDER_SETTINGS: Required<RecorderSettings> = {
  videoBitsPerSecond: 500_000,
  audioBitsPerSecond: 48_000,
};

/**
 * @param cameraStream getUserMedia 的流，取它的视频轨；没有视频轨就只录音。
 * @param mixedAudio AudioGraph.mixedStream，取它的音轨。
 */
export function recorderFactory(
  cameraStream: MediaStream,
  mixedAudio: MediaStream,
  settings: RecorderSettings = {},
): () => RecorderLike {
  return () => {
    const videoTracks = cameraStream.getVideoTracks();
    const stream = new MediaStream([...videoTracks, ...mixedAudio.getAudioTracks()]);
    const candidates = videoTracks.length > 0 ? RECORDING_MIME_CANDIDATES : AUDIO_ONLY_MIME_CANDIDATES;
    const mimeType = pickRecordingMime((mime) => MediaRecorder.isTypeSupported(mime), candidates);
    const merged = { ...DEFAULT_RECORDER_SETTINGS, ...settings };
    return new MediaRecorder(stream, {
      ...(mimeType ? { mimeType } : {}),
      audioBitsPerSecond: merged.audioBitsPerSecond,
      ...(videoTracks.length > 0 ? { videoBitsPerSecond: merged.videoBitsPerSecond } : {}),
    }) as unknown as RecorderLike;
  };
}
