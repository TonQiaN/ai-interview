/**
 * 按量出来的上行网速定录像的码率。
 *
 * 面试中候选人的声音是从上传的录像分片里取的。录像比上行快的话，分片在本机越攒越多，
 * 服务端拿到的声音落后几十秒到几分钟：面试官只能按半截回答来判断，实时对话接不上话，
 * 面试结束后候选人还要等很久才传完。所以上行装不下的时候把画面的码率降下来。
 * 声音的码率不动，语音识别靠它。
 */

/** 上行至少要是录像码率的多少倍才算够用。留出余量给重传、别的请求和网速波动。 */
export const UPLINK_HEADROOM = 1.5;

/** 画面码率最低降到这么多（比特每秒）。640×480 的人像再低就看不清人了。 */
export const MIN_VIDEO_BITS_PER_SECOND = 120_000;

export interface RecordingBitrate {
  videoBitsPerSecond: number;
  audioBitsPerSecond: number;
}

export interface FittedBitrate extends RecordingBitrate {
  /** 画面码率比服务端给的低。 */
  reduced: boolean;
  /** 按这个码率录，上行留出余量之后装得下。画面降到最低还装不下时是 false。 */
  fits: boolean;
}

/**
 * @param configured 服务端给的码率。只录声音时画面码率传 0。
 * @param uplinkKbps 设备检测时量出来的上行（千比特每秒）；没量出来是 null，按服务端给的录。
 */
export function fitRecordingBitrate(
  configured: RecordingBitrate,
  uplinkKbps: number | null,
): FittedBitrate {
  if (uplinkKbps === null) return { ...configured, reduced: false, fits: true };
  const budget = (uplinkKbps * 1000) / UPLINK_HEADROOM;
  const room = Math.floor(budget - configured.audioBitsPerSecond);
  const video = Math.min(configured.videoBitsPerSecond, Math.max(room, MIN_VIDEO_BITS_PER_SECOND));
  return {
    videoBitsPerSecond: video,
    audioBitsPerSecond: configured.audioBitsPerSecond,
    reduced: video < configured.videoBitsPerSecond,
    fits: video + configured.audioBitsPerSecond <= budget,
  };
}
