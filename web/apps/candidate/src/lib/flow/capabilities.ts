/**
 * 打开链接后先看这个浏览器能不能面试。
 *
 * 不能的情况要说清楚该怎么办（多半是「请复制链接，用系统浏览器打开」），
 * 不要等到点了「开始」才报一个看不懂的错。
 */
import { pickRecordingMime } from "../recording/mime";

export interface BrowserEnvironment {
  /** 是否是 HTTPS（或 localhost）。不是的话浏览器不给摄像头和麦克风。 */
  secureContext: boolean;
  hasGetUserMedia: boolean;
  hasMediaRecorder: boolean;
  /** 这个浏览器能录的格式；一个都不支持是 null。 */
  recordingMime: string | null;
  hasAudioContext: boolean;
  /** 没有的话录像分片只能放在内存里，刷新页面会丢还没传完的部分。 */
  hasIndexedDb: boolean;
  userAgent: string;
}

/** 会拦住面试的问题。 */
export type CapabilityProblem =
  | "insecure_context"
  | "no_media_devices"
  | "no_recorder"
  | "no_audio_context";

/** 不拦面试、但要提醒的问题。 */
export type CapabilityWarning = "no_persistent_storage";

/** 内嵌在别的应用里的浏览器。这些浏览器对摄像头、录制的支持参差不齐。 */
export type InAppBrowser = "wechat" | "wecom" | "dingtalk" | "feishu" | "qq" | "weibo" | "alipay";

export interface CapabilityReport {
  ok: boolean;
  problems: CapabilityProblem[];
  warnings: CapabilityWarning[];
  inAppBrowser: InAppBrowser | null;
  recordingMime: string | null;
}

const IN_APP_PATTERNS: Array<[InAppBrowser, RegExp]> = [
  // 企业微信的 UA 里也带 MicroMessenger，要先判断
  ["wecom", /wxwork/i],
  ["wechat", /MicroMessenger/i],
  ["dingtalk", /DingTalk/i],
  ["feishu", /Lark|Feishu/i],
  ["qq", /\bQQ\//i],
  ["weibo", /Weibo/i],
  ["alipay", /AlipayClient/i],
];

export function detectInAppBrowser(userAgent: string): InAppBrowser | null {
  for (const [name, pattern] of IN_APP_PATTERNS) {
    if (pattern.test(userAgent)) return name;
  }
  return null;
}

export function checkCapabilities(env: BrowserEnvironment): CapabilityReport {
  const problems: CapabilityProblem[] = [];
  if (!env.secureContext) problems.push("insecure_context");
  if (!env.hasGetUserMedia) problems.push("no_media_devices");
  if (!env.hasMediaRecorder || env.recordingMime === null) problems.push("no_recorder");
  if (!env.hasAudioContext) problems.push("no_audio_context");

  const warnings: CapabilityWarning[] = [];
  if (!env.hasIndexedDb) warnings.push("no_persistent_storage");

  return {
    ok: problems.length === 0,
    problems,
    warnings,
    inAppBrowser: detectInAppBrowser(env.userAgent),
    recordingMime: env.recordingMime,
  };
}

/** 读当前浏览器的情况。只能在浏览器里调用。 */
export function readBrowserEnvironment(): BrowserEnvironment {
  const hasMediaRecorder = typeof MediaRecorder !== "undefined";
  return {
    secureContext: window.isSecureContext,
    hasGetUserMedia: typeof navigator.mediaDevices?.getUserMedia === "function",
    hasMediaRecorder,
    recordingMime: hasMediaRecorder ? pickRecordingMime((type) => MediaRecorder.isTypeSupported(type)) : null,
    hasAudioContext: typeof AudioContext !== "undefined",
    hasIndexedDb: typeof indexedDB !== "undefined",
    userAgent: navigator.userAgent,
  };
}
