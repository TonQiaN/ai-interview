/**
 * 选 MediaRecorder 的录制格式。
 *
 * 不同浏览器支持的容器和编码不一样：Chromium 系（含安卓微信）用 WebM，Safari 和 iOS 用 MP4。
 * 按下面的顺序挑第一个浏览器支持的；都不支持就不指定，交给浏览器用默认格式。
 */

/**
 * 优先顺序。
 * - VP8 排在 VP9 前面：手机上 VP9 软编码耗电、容易掉帧，初筛面试不需要那点画质。
 * - H.264 的 WebM 排在后面：不是所有播放器都认。
 * - MP4 留给 Safari。
 */
export const RECORDING_MIME_CANDIDATES = [
  "video/webm;codecs=vp8,opus",
  "video/webm;codecs=vp9,opus",
  "video/webm;codecs=h264,opus",
  "video/webm",
  "video/mp4;codecs=avc1.42E01E,mp4a.40.2",
  "video/mp4;codecs=h264,aac",
  "video/mp4",
] as const;

/** 只录音（候选人没有摄像头，而这场面试允许不开摄像头）时的顺序。 */
export const AUDIO_ONLY_MIME_CANDIDATES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/mp4;codecs=mp4a.40.2",
  "audio/mp4",
] as const;

/**
 * @returns 选中的格式；浏览器一个都不认时返回 null（调用方不传 mimeType，用浏览器默认）。
 */
export function pickRecordingMime(
  isTypeSupported: (mime: string) => boolean,
  candidates: readonly string[] = RECORDING_MIME_CANDIDATES,
): string | null {
  for (const candidate of candidates) {
    try {
      if (isTypeSupported(candidate)) return candidate;
    } catch {
      // 个别旧浏览器对不认识的写法会抛错，当作不支持
    }
  }
  return null;
}

/** 从 MIME 得到容器名，用于上报和文件扩展名。 */
export function containerOf(mime: string): "webm" | "mp4" | "unknown" {
  const base = mime.split(";")[0]?.trim().toLowerCase() ?? "";
  if (base.endsWith("/webm")) return "webm";
  if (base.endsWith("/mp4")) return "mp4";
  return "unknown";
}
