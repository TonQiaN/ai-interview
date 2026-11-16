/**
 * 页面上的中文文字。英文的那一份在 `en.ts`，两份的结构必须一样（类型会检查）。
 * 文字集中放在这里，改措辞不用翻组件。
 */
import type { UtteranceKind } from "../lib/api/room-types";
import type { CapabilityProblem, InAppBrowser } from "../lib/flow/capabilities";
import type { MediaErrorKind } from "../lib/media/errors";
import type { ProblemKind } from "../session/interview-session";

const inAppBrowser: Record<InAppBrowser, string> = {
  wechat: "微信",
  wecom: "企业微信",
  dingtalk: "钉钉",
  feishu: "飞书",
  qq: "QQ",
  weibo: "微博",
  alipay: "支付宝",
};

const problems: Record<
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

const capabilityProblems: Record<CapabilityProblem, string> = {
  insecure_context: "页面不是通过安全连接（https）打开的。",
  no_media_devices: "浏览器不允许网页使用摄像头和麦克风。",
  no_recorder: "浏览器不支持录制。",
  no_audio_context: "浏览器不支持播放和处理声音。",
};

/** 申请摄像头或麦克风失败时的说明：发生了什么，该怎么办。 */
const mediaErrors: Record<MediaErrorKind, { title: string; steps: string[] }> = {
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

const utteranceKind: Record<UtteranceKind, string> = {
  opening: "开场",
  question: "提问",
  follow_up: "追问",
  transition: "过渡",
  closing: "结束语",
  other: "",
};

/** 结束页常用的一句。 */
const SAVED = "已经回答的内容已保存并提交。";

export const zh = {
  /** 写在 `<html lang>` 上。 */
  htmlLang: "zh-CN",
  /** 浏览器标签页上的标题。 */
  pageTitle: "AI 面试",
  loading: "正在打开面试",
  retry: "重试",
  copyLink: "复制链接",
  cameraLabel: "你的画面",
  /** 须知、同意、设备、开始四步的名字。 */
  steps: ["须知", "同意", "设备", "开始"],
  minutes: (seconds: number): string => `约 ${Math.max(1, Math.round(seconds / 60))} 分钟`,
  /**
   * 服务端拒绝时带回来的说明是中文的，中文页面直接显示；
   * 别的语言换成一句通用的话。
   */
  otherError: (message: string): string => message,
  inAppBrowser,

  welcome: {
    company: "AI 面试",
    hello: (name: string | null | undefined): string => (name ? `${name}，你好` : "你好"),
    intro: (position: string, duration: string): string =>
      `欢迎参加「${position}」的 AI 面试。这是第一轮面试，由 AI 面试官提问，你口头回答，${duration}。`,
    start: "开始准备",
    resume: "检查设备，继续面试",
    resuming: "你有一场还没完成的面试。检查设备之后，会从上次中断的地方继续。",
    inAppTitle: (app: string): string => `你正在${app}里打开`,
    inAppBody: "内置浏览器录音录像不稳定。建议复制链接，到 Safari、Chrome 或 Edge 里打开。",
    volatileStorage: "当前是无痕或隐私模式，面试中请不要刷新页面，否则还没上传的录像会丢失。",
    notes: {
      time: {
        lead: "留出完整的时间。",
        rest: (duration: string): string => `面试${duration}，开始后不能暂停。`,
      },
      quiet: { lead: "找一个安静的地方。", rest: "戴耳机效果更好，面试官听得更清楚。" },
      recording: {
        leadWithCamera: "全程录音录像。",
        leadCameraOptional: "全程录音，有摄像头时同时录像。",
        rest: "录下的内容只用于这次招聘的评估，下一步会请你确认。",
      },
      network: {
        lead: "保持网络稳定。",
        rest: "中途断网不用慌，录像会先存在本机，恢复后自动上传。",
      },
    },
  },

  consent: {
    title: "开始之前，请确认",
    hint: "点右边的箭头可以看每一项的详细说明。全部勾选后才能继续。",
    agree: "同意并继续",
    decline: "如果你不同意其中任何一项，可以直接关闭页面，并联系招聘人员安排其他面试方式。",
    networkError: "网络不稳定，没有提交成功，请再试一次。",
  },

  device: {
    title: "检查设备",
    hintGranted: "确认画面和声音都正常，再开始面试。",
    hintAsk: "点下面的按钮后，浏览器会询问是否允许使用摄像头和麦克风，请选择「允许」。",
    next: "下一步",
    nextWithoutMic: "麦克风没反应，仍然继续",
    allow: "允许使用摄像头和麦克风",
    recheck: "重新检测",
    camera: {
      title: "摄像头",
      ok: "能看到自己就可以了。请让脸在画面中间，光线尽量从正面来。",
      missing: "没有找到摄像头，这场面试将只录音。",
      required: "面试全程录像。",
      optional: "有摄像头时会录像，没有也可以面试。",
    },
    mic: {
      title: "麦克风",
      heard: "听到你的声音了。",
      speak: "请说一句话，比如「你好，我准备好了」。",
      purpose: "面试官通过麦克风听你的回答。",
      slow: "一直没有听到声音。请检查麦克风有没有被静音、有没有选对设备，或者靠近一点说话。",
    },
    speaker: {
      title: "扬声器或耳机",
      ok: "声音正常。",
      played: "听到「叮」的一声了吗？没听到的话，调大音量、关掉静音再试一次。",
      purpose: "播放一段提示音，确认你能听到面试官的声音。",
      play: "播放提示音",
      playAgain: "再播一次",
      heard: "听到了",
    },
    network: {
      title: "网络",
      retest: "重新测速",
      testing: "正在测上传速度，面试的录像要边录边传。",
      failed: "没有测出上传速度，可能是网络不稳定。可以继续，建议先确认网络正常。",
      ok: "上传速度够用。",
      slow: "上传速度偏慢，可以面试，但结束后要多等一会儿才能传完录像。建议换一个网络，比如靠近路由器，或者改用手机流量。",
      slowReduced:
        "上传速度偏慢，为了不耽误面试，录像的画面会录得模糊一些。想让画面清楚一些，可以换一个网络，比如靠近路由器，或者改用手机流量。",
      slowRealtime:
        "上传速度偏慢，面试官的回应可能会变慢，结束后还要多等一会儿才能传完录像。建议换一个网络，比如靠近路由器，或者改用手机流量。",
    },
    mediaErrors,
  },

  ready: {
    title: "准备好就开始",
    titleResume: "准备好就继续",
    begin: "开始面试",
    resume: "继续面试",
    networkError: "网络不稳定，没能开始。请检查网络后再点一次。",
    summary: (position: string, duration: string): string => `「${position}」· ${duration}`,
    rules: (silenceSec: number): string[] => [
      "面试官会一题一题地问。听完题目后直接开口回答，不用点任何按钮。",
      `答完后停顿 ${silenceSec} 秒，会自动进入下一题。想快一点，可以点「我答完了」。`,
      "中间想一想再说没关系，只要还在说话就不会被打断。",
      "请保持正对镜头，不要切到别的应用或页面。",
      "如果页面意外关闭或刷新，重新打开链接可以从中断的地方继续。",
    ],
  },

  interview: {
    recording: "录制中",
    elapsed: "已用时",
    end: "结束",
    questionOf: (index: number, total: number): string => `第 ${index} 题 / 共 ${total} 题`,
    utteranceKind,
    hints: {
      preparing: "正在准备下一题",
      submitting: "好的，稍等",
      reconnecting: "正在重新连接",
      asking: "请听题，听完直接回答",
      reading: "语音没有加载出来，请看文字，准备好后点「开始回答」",
      paused: "已暂停",
      noSpeech: "请开始回答。如果已经在说，请靠近麦克风",
      maxWarning: "这道题的时间快到了，请尽快收尾",
      countdown: "说完了吗？还想补充就继续说",
      answer: "请回答",
    },
    notDone: "我还没说完",
    done: "我答完了，下一题",
    replay: "再听一遍题目",
    startAnswer: "开始回答",
    deviceLost: {
      title: "摄像头或麦克风断开了",
      action: "重新连接",
      body: "面试已暂停，计时也停了。接好设备后点「重新连接」继续。",
      failed: "还是连不上。请检查设备有没有被拔掉或被别的应用占用，然后再试。",
    },
    audioHalted: {
      title: "声音被暂停了",
      action: "继续面试",
      body: "可能是来了电话，或者刚才切到了别的应用。面试已暂停，计时也停了，点「继续面试」接着进行。",
      failed: "声音还是没有恢复。请先挂断电话或关掉正在用麦克风的应用，再点「继续面试」。",
    },
    offline: {
      title: "网络不稳定",
      body: "正在重新连接。录像先存在本机，不会丢，网络恢复后自动上传。",
    },
    backlog: "网络较慢，部分录像还在上传。面试可以正常进行。",
    leave: {
      title: "现在结束面试？",
      body: "结束之后不能再回来继续。已经回答的内容会保存并提交给招聘人员。",
      stay: "继续面试",
      confirm: "结束面试",
    },
  },

  finishing: {
    title: "面试结束了，正在保存录像",
    description: "请先不要关闭这个页面。保存完成后会有提示，一般不到一分钟。",
    failedTitle: "录像没有保存成功",
    failedBody: "请检查网络后点「重试」。录像还在本机，不会丢。",
    progress: "录像保存进度",
    offlineTitle: "网络断开了",
    offlineBody: "恢复后会自动继续，请保持页面打开。",
    saving: "保存中",
    slowDescription:
      "你的网络上传比较慢，录像还要一些时间才能保存完。请保持这个页面打开，完成后会有提示。",
    remaining: (size: string): string => `还剩 ${size}`,
    remainingWithWait: (size: string, wait: string): string => `还剩 ${size}，${wait}`,
    waitUnderMinute: "不到一分钟",
    waitMinutes: (minutes: number): string => `大约还要 ${minutes} 分钟`,
    waitHours: (hours: number, minutes: number): string =>
      minutes > 0 ? `大约还要 ${hours} 小时 ${minutes} 分钟` : `大约还要 ${hours} 小时`,
    movedTitle: "面试已经在另一个页面里继续了",
    movedDescription: "这个页面已经停止录制，正在保存它录下的部分。请先不要关闭这个页面。",
  },

  done: {
    aborted: {
      title: "面试已中止",
      description: "这条面试链接已经失效，面试没能继续。如有疑问，请联系招聘人员。",
    },
    moved: {
      title: "面试已经在另一个页面里继续了",
      description:
        "同一条面试链接又在别的页面打开了，面试换到了那边。这个页面已经停止录制，它录下的部分已经保存。请回到那个页面继续面试，这个页面可以关掉了。",
    },
    resumeHere: "回到这个页面继续面试",
    left: { title: "面试已结束", description: `${SAVED}现在可以关闭这个页面了。` },
    noResponse: {
      title: "面试已结束",
      description: `我们一直没有听到你的回答，面试先结束了。${SAVED}如果是设备出了问题，请联系招聘人员重新安排。`,
    },
    connectionLost: {
      title: "面试已中断",
      description: `页面离开太久，面试已经结束。${SAVED}需要重新安排的话，请联系招聘人员。`,
    },
    unknown: { title: "面试已结束", description: "录像已经保存。现在可以关闭这个页面了。" },
    completed: {
      title: "面试完成，谢谢你",
      description: "录像已经保存。招聘人员看过之后会联系你。现在可以关闭这个页面了。",
    },
  },

  /** 页面渲染到一半出错时换上的那一页。脚本没加载出来时的提示在 index.html 里，两处的说法要对得上。 */
  crash: {
    title: "页面出了点问题",
    description:
      "请刷新页面。已经录下和回答过的内容都保存着；面试还没结束的话，重新检查设备后会从中断的地方继续。",
    reload: "刷新页面",
  },

  problem: {
    unsupportedTitle: "这个浏览器不支持面试",
    unsupportedBody: "请用最新版的 Chrome、Edge 或 Safari 打开这条链接。",
    inAppTitle: "请换到系统浏览器打开",
    inAppBody: (app: string): string =>
      `${app}里的内置浏览器不能录音录像。请复制链接，到 Safari、Chrome 或 Edge 里粘贴打开。`,
    detected: "检测到的问题",
    capabilityProblems,
    problems,
  },
};

/** 一种语言的全部文字。 */
export type Texts = typeof zh;
