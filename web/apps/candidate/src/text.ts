/**
 * 页面上的文案。集中放在这里，改措辞不用翻组件。
 */
import type { InAppBrowser } from "./lib/flow/capabilities";
import type { MediaErrorKind } from "./lib/media/errors";
import type { ProblemKind } from "./session/interview-session";

export const IN_APP_BROWSER_NAME: Record<InAppBrowser, string> = {
  wechat: "微信",
  wecom: "企业微信",
  dingtalk: "钉钉",
  feishu: "飞书",
  qq: "QQ",
  weibo: "微博",
  alipay: "支付宝",
};

export const PROBLEM_TEXT: Record<
  Exclude<ProblemKind, "unsupported">,
  { title: string; description: string }
> = {
  missing_link: {
    title: "链接不完整",
    description: "这个地址里缺少面试信息。请直接点击招聘人员发给你的完整链接，不要手动输入或截断。",
  },
  invalid_link: {
    title: "链接无效",
    description: "没有找到这条面试链接。请确认链接完整，或联系发给你链接的招聘人员重新获取。",
  },
  expired: {
    title: "链接已过期",
    description: "这条面试链接已经过了有效期。请联系发给你链接的招聘人员，重新获取。",
  },
  revoked: {
    title: "链接已失效",
    description: "这条面试链接已被停用。如果有疑问，请联系发给你链接的招聘人员。",
  },
  network: {
    title: "连不上服务器",
    description: "请检查网络后重试。如果你在公司网络或开着代理，可以换成手机热点试试。",
  },
  error: {
    title: "出了点问题",
    description: "页面遇到了意外的错误。请刷新页面重试；如果一直这样，请联系招聘人员。",
  },
};

/** 申请摄像头或麦克风失败时的说明：发生了什么，该怎么办。 */
export const MEDIA_ERROR_TEXT: Record<MediaErrorKind, { title: string; steps: string[] }> = {
  permission_denied: {
    title: "没有拿到摄像头或麦克风的权限",
    steps: [
      "点地址栏左边的图标（锁或设置），把摄像头和麦克风改成「允许」。",
      "手机上还要在系统设置里允许这个浏览器使用摄像头和麦克风。",
      "改好之后点下面的「重新检测」。",
    ],
  },
  device_not_found: {
    title: "没有找到摄像头或麦克风",
    steps: [
      "确认设备已经接好；用耳机的话检查耳机上的麦克风。",
      "台式机可能没有内置摄像头，可以换用手机或笔记本。",
    ],
  },
  device_in_use: {
    title: "摄像头或麦克风被别的应用占用了",
    steps: [
      "关掉正在使用摄像头或麦克风的应用（视频会议、相机、其他浏览器标签页）。",
      "然后点「重新检测」。",
    ],
  },
  insecure_context: {
    title: "当前页面不能使用摄像头和麦克风",
    steps: ["请确认打开的是招聘人员发来的完整链接（以 https 开头）。"],
  },
  unsupported: {
    title: "这个浏览器不支持摄像头或麦克风",
    steps: ["请换用最新版的 Chrome、Edge 或 Safari 打开链接。"],
  },
  constraints_unsatisfied: {
    title: "摄像头不符合要求",
    steps: ["请换一个摄像头，或换一台设备再试。"],
  },
  aborted: {
    title: "授权被打断了",
    steps: ["请再点一次「重新检测」，并在弹出的提示里选择「允许」。"],
  },
  unknown: {
    title: "打开摄像头或麦克风时出错",
    steps: ["请刷新页面再试一次。", "还是不行的话，换一个浏览器或设备。"],
  },
};

export function minutesText(seconds: number): string {
  return `约 ${Math.max(1, Math.round(seconds / 60))} 分钟`;
}
