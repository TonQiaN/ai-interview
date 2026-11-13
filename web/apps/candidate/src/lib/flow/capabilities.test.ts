import { describe, expect, it } from "vitest";
import { type BrowserEnvironment, checkCapabilities, detectInAppBrowser } from "./capabilities";

const good: BrowserEnvironment = {
  secureContext: true,
  hasGetUserMedia: true,
  hasMediaRecorder: true,
  recordingMime: "video/webm;codecs=vp8,opus",
  hasAudioContext: true,
  hasIndexedDb: true,
  userAgent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
};

describe("浏览器能力检查", () => {
  it("正常的桌面浏览器可以面试", () => {
    expect(checkCapabilities(good)).toEqual({
      ok: true,
      problems: [],
      warnings: [],
      inAppBrowser: null,
      recordingMime: "video/webm;codecs=vp8,opus",
    });
  });

  it("不是 HTTPS 时拿不到摄像头和麦克风", () => {
    const report = checkCapabilities({ ...good, secureContext: false, hasGetUserMedia: false });
    expect(report.ok).toBe(false);
    expect(report.problems).toEqual(["insecure_context", "no_media_devices"]);
  });

  it("有 MediaRecorder 但一种格式都录不了，也算不能录", () => {
    expect(checkCapabilities({ ...good, recordingMime: null }).problems).toEqual(["no_recorder"]);
    expect(checkCapabilities({ ...good, hasMediaRecorder: false, recordingMime: null }).problems).toEqual(["no_recorder"]);
  });

  it("没有 IndexedDB 只提醒，不拦", () => {
    const report = checkCapabilities({ ...good, hasIndexedDb: false });
    expect(report.ok).toBe(true);
    expect(report.warnings).toEqual(["no_persistent_storage"]);
  });

  it("认出内嵌在聊天软件里的浏览器", () => {
    const ios = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
    expect(detectInAppBrowser(`${ios} MicroMessenger/8.0.50(0x18003233) NetType/WIFI Language/zh_CN`)).toBe("wechat");
    expect(detectInAppBrowser(`${ios} wxwork/4.1.30 MicroMessenger/7.0.1 Language/zh`)).toBe("wecom");
    expect(detectInAppBrowser(`${ios} AliApp(DingTalk/7.6.0) com.laiwang.DingTalk/1 DingTalk/7.6.0`)).toBe("dingtalk");
    expect(detectInAppBrowser(`${ios} Lark/7.20.0 LarkLocale/zh_CN`)).toBe("feishu");
    expect(detectInAppBrowser(`${ios} QQ/9.0.0.1 NetType/4G`)).toBe("qq");
    expect(detectInAppBrowser(`${ios} AlipayClient/10.6.0`)).toBe("alipay");
    expect(detectInAppBrowser(good.userAgent)).toBeNull();
    expect(detectInAppBrowser(`${ios} Version/18.0 Safari/604.1`)).toBeNull();
  });
});
