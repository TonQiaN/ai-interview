/** 把数据变成界面上的文字。时间都按浏览器的本地时区显示。 */
import type { Tone } from "@ai-interview/ui";
import type {
  ClaimVerdict,
  CommunicationAspect,
  ComponentState,
  Credibility,
  Decision,
  FocusVerdict,
  InterviewStatus,
  InviteStatus,
  Level,
  ParseStatus,
  PositionStatus,
  Recommendation,
  ReportStatus,
  RequirementVerdict,
  ReviewFlagCode,
  RiskLevel,
  SynthStatus,
} from "../api/types";

export const statusLabel: Record<InterviewStatus, string> = {
  draft: "草稿",
  preparing: "准备中",
  prepare_failed: "准备失败",
  ready: "待发送",
  invited: "已发链接",
  in_progress: "面试中",
  completed: "已完成",
  abandoned: "中途离开",
  expired: "链接过期",
  cancelled: "已取消",
};

export const statusTone: Record<InterviewStatus, Tone> = {
  draft: "neutral",
  preparing: "info",
  prepare_failed: "danger",
  ready: "info",
  invited: "info",
  in_progress: "warning",
  completed: "success",
  abandoned: "warning",
  expired: "neutral",
  cancelled: "neutral",
};

/** 每个状态下 HR 接下来该做什么 */
export const statusHint: Record<InterviewStatus, string> = {
  draft: "设置好之后生成题目。",
  preparing: "正在出题并合成面试官的语音，一般一两分钟。",
  prepare_failed: "题目或语音没有准备好，可以重试。",
  ready: "题目和语音都好了，可以生成链接发给候选人。",
  invited: "链接已经生成，等候选人来面试。",
  in_progress: "候选人正在面试。",
  completed: "面试已经结束，可以回看和查看报告。",
  abandoned: "候选人中途离开，没有回来。已有的部分可以回看。",
  expired: "链接过期了，候选人没有来。",
  cancelled: "这场面试已取消。",
};

export const recommendationLabel: Record<Recommendation, string> = {
  strong_advance: "强烈建议推进",
  advance: "建议推进",
  borderline: "拿不准",
  not_advance: "不建议推进",
};

export const recommendationTone: Record<Recommendation, Tone> = {
  strong_advance: "success",
  advance: "success",
  borderline: "warning",
  not_advance: "danger",
};

export const decisionLabel: Record<Decision, string> = {
  advance: "进入下一轮",
  reject: "不推进",
  undecided: "还没定",
};
export const decisionTone: Record<Decision, Tone> = {
  advance: "success",
  reject: "danger",
  undecided: "neutral",
};

export const reportStatusLabel: Record<ReportStatus, string> = {
  pending: "排队中",
  running: "生成中",
  done: "已生成",
  failed: "生成失败",
};
export const parseStatusLabel: Record<ParseStatus, string> = {
  pending: "排队中",
  running: "解析中",
  done: "已解析",
  failed: "解析失败",
};
export const parseStatusTone: Record<ParseStatus, Tone> = {
  pending: "neutral",
  running: "info",
  done: "success",
  failed: "danger",
};
export const synthStatusLabel: Record<SynthStatus, string> = {
  pending: "待合成",
  running: "合成中",
  done: "已合成",
  failed: "合成失败",
};
export const synthStatusTone: Record<SynthStatus, Tone> = {
  pending: "neutral",
  running: "info",
  done: "success",
  failed: "danger",
};
export const inviteStatusLabel: Record<InviteStatus, string> = {
  active: "有效",
  used: "已使用",
  expired: "已过期",
  revoked: "已作废",
};
export const inviteStatusTone: Record<InviteStatus, Tone> = {
  active: "success",
  used: "info",
  expired: "neutral",
  revoked: "neutral",
};

// ───────── 分析报告 ─────────
// 下面这些键和 crates/interview-report/src/report.rs 里的枚举一一对应，
// 有测试拿样例报告核对：出现了这里没有的值会报错。

export const levelLabel: Record<Level, string> = { low: "低", medium: "中", high: "高" };
export const confidenceLabel: Record<Level, string> = {
  low: "把握不大",
  medium: "把握一般",
  high: "把握较大",
};

export const riskLabel: Record<RiskLevel, string> = {
  none: "没有发现",
  low: "低",
  medium: "中",
  high: "高",
};
export const riskTone: Record<RiskLevel, Tone> = {
  none: "success",
  low: "success",
  medium: "warning",
  high: "danger",
};
/** 严重程度、可靠程度用同一套颜色：越高越醒目 */
export const levelTone: Record<Level, Tone> = { low: "neutral", medium: "warning", high: "danger" };

export const focusVerdictLabel: Record<FocusVerdict, string> = {
  satisfied: "达到",
  partially_satisfied: "部分达到",
  not_satisfied: "没达到",
  not_assessed: "没考察到",
};
export const focusVerdictTone: Record<FocusVerdict, Tone> = {
  satisfied: "success",
  partially_satisfied: "warning",
  not_satisfied: "danger",
  not_assessed: "neutral",
};

export const requirementVerdictLabel: Record<RequirementVerdict, string> = {
  met: "符合",
  partially_met: "部分符合",
  not_met: "不符合",
  unknown: "看不出来",
};
export const requirementVerdictTone: Record<RequirementVerdict, Tone> = {
  met: "success",
  partially_met: "warning",
  not_met: "danger",
  unknown: "neutral",
};
export const importanceLabel: Record<"must_have" | "nice_to_have", string> = {
  must_have: "必须",
  nice_to_have: "加分",
};
export const basisLabel: Record<"resume" | "interview" | "both" | "none", string> = {
  resume: "依据简历",
  interview: "依据面试",
  both: "依据简历和面试",
  none: "没有依据",
};
export const matchLevelLabel: Record<Level, string> = {
  low: "匹配度低",
  medium: "匹配度中等",
  high: "匹配度高",
};
export const matchLevelTone: Record<Level, Tone> = {
  low: "danger",
  medium: "warning",
  high: "success",
};

export const credibilityLabel: Record<Credibility, string> = {
  high: "可信度高",
  medium: "可信度中等",
  low: "可信度低",
  insufficient: "依据不够，没法判断",
};
export const credibilityTone: Record<Credibility, Tone> = {
  high: "success",
  medium: "warning",
  low: "danger",
  insufficient: "neutral",
};

export const claimVerdictLabel: Record<ClaimVerdict, string> = {
  corroborated: "对得上",
  partially_corroborated: "部分对得上",
  not_covered: "没问到",
  doubtful: "存疑",
  contradicted: "对不上",
};
export const claimVerdictTone: Record<ClaimVerdict, Tone> = {
  corroborated: "success",
  partially_corroborated: "warning",
  not_covered: "neutral",
  doubtful: "warning",
  contradicted: "danger",
};

export const signalKindLabel: Record<string, string> = {
  scripted_answer: "像在念稿",
  timing_pattern: "答题节奏异常",
  depth_mismatch: "深浅不一致",
  page_event: "页面操作",
  third_party: "疑似有其他人",
  identity: "身份存疑",
  manipulation: "试图影响面试官",
  other: "其他",
};

export const aspectLabel: Record<CommunicationAspect, string> = {
  clarity: "表达清楚",
  structure: "条理",
  responsiveness: "切题",
  composure: "情绪稳定",
};

export const reviewFlagLabel: Record<ReviewFlagCode, string> = {
  integrity_risk: "诚信风险",
  must_have_unmet: "必须项不符合",
  resume_doubt: "简历存疑",
  weak_evidence: "依据偏少",
  incomplete_analysis: "分析不完整",
};

const stageLabel: Record<string, string> = {
  requirements: "岗位要求对照",
  resume: "简历核实",
  integrity: "诚信观察",
  communication: "表达和状态",
  synthesis: "综合结论",
};

/** 报告是分几步生成的，这是每一步的名字。question:q3 这样的是某一道题的点评 */
export function stageName(stage: string): string {
  if (stage.startsWith("question:"))
    return `第 ${stage.slice("question:".length).replace(/^q/, "")} 题的点评`;
  return stageLabel[stage] ?? stage;
}

export const speakerLabel: Record<string, string> = {
  interviewer: "面试官",
  candidate: "候选人",
  other: "其他人",
};

export const endReasonLabel: Record<string, string> = {
  finished: "正常结束",
  early_finish: "面试官提前结束",
  left: "候选人离开",
  timeout: "超时",
};

// ───────── 过程事件 ─────────
// 事件种类的清单在 docs/architecture/api.md。

export const eventLabel: Record<string, string> = {
  device_check: "设备检测",
  visibility_hidden: "切出页面",
  visibility_visible: "回到页面",
  window_blur: "窗口失去焦点",
  window_focus: "窗口回到焦点",
  fullscreen_exit: "退出全屏",
  paste: "粘贴",
  copy: "复制",
  device_changed: "更换了设备",
  network_offline: "断网",
  network_online: "网络恢复",
  ws_reconnected: "重新连上",
  recording_gap: "录制中断",
  face_count_changed: "画面里的人数变了",
};

export type EventSeverity = "info" | "notice" | "warning";

/** 事件在时间轴上有多醒目。只是提示 HR 去看，不代表结论 */
export function eventSeverity(kind: string): EventSeverity {
  if (["visibility_hidden", "paste", "face_count_changed", "fullscreen_exit"].includes(kind))
    return "warning";
  if (["window_blur", "copy", "device_changed", "network_offline", "recording_gap"].includes(kind))
    return "notice";
  return "info";
}

type EventLike = { kind: string; duration_ms: number | null; detail: string | null };

/** 事件除了种类之外的补充：持续了多久、附带的说明。没有就是空 */
export function eventNote(event: EventLike): string | null {
  if (event.kind === "face_count_changed" && event.detail) return `变成 ${event.detail} 个人`;
  const parts: string[] = [];
  if (event.duration_ms) parts.push(`持续 ${duration(event.duration_ms)}`);
  if (event.detail) parts.push(event.detail);
  return parts.length > 0 ? parts.join("，") : null;
}

/** 事件的一句话说明：种类加补充 */
export function eventText(event: EventLike): string {
  const name = eventLabel[event.kind] ?? event.kind;
  const note = eventNote(event);
  return note ? `${name}（${note}）` : name;
}

export const languageLabel: Record<string, string> = {
  zh: "中文",
  en: "英文",
  ja: "日文",
  es: "西班牙文",
  ar: "阿拉伯文",
};

/** 毫秒 → 12:36 或 1:02:05 */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** 毫秒 → 「24 分钟」「48 秒」 */
export function duration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "—";
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec} 秒`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min} 分钟`;
  return `${Math.floor(min / 60)} 小时 ${min % 60} 分钟`;
}

const dateFmt = new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" });
const timeFmt = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});
const fullFmt = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** 「刚刚」「12 分钟前」「今天 14:05」「昨天 09:30」「11月9日」；将来的时间写成「3 天后」 */
export function relativeTime(isoString: string | null | undefined, now: Date = new Date()): string {
  if (!isoString) return "—";
  const d = new Date(isoString);
  const diff = now.getTime() - d.getTime();
  if (diff < 0) {
    const ahead = -diff;
    if (ahead < 3_600_000) return `${Math.max(1, Math.round(ahead / 60_000))} 分钟后`;
    if (ahead < 86_400_000) return `${Math.round(ahead / 3_600_000)} 小时后`;
    return `${Math.round(ahead / 86_400_000)} 天后`;
  }
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (days === 0) return `今天 ${timeFmt.format(d)}`;
  if (days === 1) return `昨天 ${timeFmt.format(d)}`;
  if (days < 7) return `${days} 天前`;
  return dateFmt.format(d);
}

export function fullTime(isoString: string | null | undefined): string {
  return isoString ? fullFmt.format(new Date(isoString)) : "—";
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** 1 到 5 分 → 「4 分」；没考察到写「没考察到」 */
export function rating(value: number | null): string {
  return value === null ? "没考察到" : `${value} 分`;
}

/** 分数用什么颜色：80 以上绿，60 以上金，再低红 */
export function scoreColor(score: number): string {
  if (score >= 80) return "var(--oil-success)";
  if (score >= 60) return "var(--oil-gold)";
  return "var(--oil-danger)";
}

/** 一年月写成「2023.03」，空的写成「至今」 */
export function yearMonth(value: string | null): string {
  return value ? value.replace("-", ".") : "至今";
}

// ───────── 系统状态 ─────────

/** 后端按名字报各个依赖的状态，这里是名字对应的中文 */
export const componentLabel: Record<string, string> = {
  database: "数据库",
  llm: "大模型（DeepSeek）",
  tts: "语音合成",
  stt: "语音识别",
  raas: "RAAS",
  storage: "文件存储",
  jobs: "后台任务",
};

export const componentStateLabel: Record<ComponentState, string> = {
  ok: "正常",
  degraded: "部分可用",
  down: "不可用",
  not_configured: "没有配置",
};

export const componentStateTone: Record<ComponentState, Tone> = {
  ok: "success",
  degraded: "warning",
  down: "danger",
  not_configured: "neutral",
};

export const positionStatusLabel: Record<PositionStatus, string> = {
  open: "在招",
  paused: "暂停",
  closed: "已关闭",
};

export const positionStatusTone: Record<PositionStatus, Tone> = {
  open: "success",
  paused: "warning",
  closed: "neutral",
};
