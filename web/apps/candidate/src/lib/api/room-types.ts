/**
 * 候选人面试间协议的类型。
 *
 * 协议的权威说明在 docs/architecture/api.md 的「面试间协议」，后端实现在 crates/interview-room。
 * 字段名和后端的 JSON 一致（snake_case）。
 */
import type { RecordingTrack } from "../upload/chunk-store";

/** 面试模式：逐题（题目和语音提前准备好）或实时对话。 */
export type InterviewMode = "scripted" | "realtime";

/** 候选人看到的会话阶段。 */
export type RoomStage =
  /** 还没点「开始面试」。 */
  | "not_started"
  /** 面试进行中（刷新或换设备后回来也是这个状态）。 */
  | "in_progress"
  /** 面试已经结束，链接不能再用来面试，但还可以补传没传完的录像。 */
  | "finished";

export interface ConsentItem {
  key: string;
  title: string;
  /** 详细说明。 */
  body: string;
  /** 必须同意才能继续。 */
  required: boolean;
}

export interface RecordingSettings {
  /** 多久切一个分片（毫秒）。 */
  timeslice_ms: number;
  video_bits_per_second: number;
  audio_bits_per_second: number;
  width: number;
  height: number;
  frame_rate: number;
}

export interface RoomSession {
  stage: RoomStage;
  /** 岗位名称。 */
  position_title: string;
  /** 公司或团队的展示名；没有配置时是 null。 */
  company_name: string | null;
  /** 对候选人的称呼；没有时是 null。 */
  candidate_name: string | null;
  mode: InterviewMode;
  /** 预计时长（秒）。 */
  expected_duration_sec: number;
  /** 这场面试是否必须开摄像头。 */
  camera_required: boolean;
  consent: {
    version: string;
    items: ConsentItem[];
    /** 之前是否已经同意过这个版本。 */
    agreed: boolean;
  };
  recording: RecordingSettings;
  /** 默认的作答设置，用来在开始前告诉候选人规则。每道题实际用的以「下一步」里的为准。 */
  answer: AnswerSettings;
  /** 面试已经进行了多少毫秒；还没开始是 null。 */
  elapsed_ms: number | null;
  /** 续面时接下来该做的一步；还没开始或已结束是 null。 */
  resume: NextStep | null;
  /** 链接什么时候过期（RFC 3339）。 */
  expires_at: string;
}

export type UtteranceKind = "opening" | "question" | "follow_up" | "transition" | "closing" | "other";

export interface Utterance {
  key: string;
  kind: UtteranceKind;
  text: string;
  /** 语音的地址；没有合成出语音时是 null，页面只显示文字。 */
  audio_url: string | null;
  audio_duration_ms: number | null;
}

export interface AnswerSettings {
  /** 开口之后连续静音多少秒算答完。 */
  silence_sec: number;
  /** 最长作答时间（秒）。 */
  max_sec: number;
  /** 至少听这么多秒才允许因为静音结束。 */
  min_sec: number;
}

export interface Progress {
  /** 当前是第几题（从 1 开始）；开场白等不算题的是 0。 */
  index: number;
  total: number;
  elapsed_ms: number;
  expected_total_ms: number;
}

/** 服务端告诉页面的「下一步」。 */
export type NextStep =
  | {
      action: "say";
      /**
       * 这一句在本场面试里的序号，每发一句加一。提交时原样带回，服务端按它判断是不是重复提交。
       * 同一句话（同一个 utterance.key）可能说两次，比如没听到回答时再问一遍，所以不能拿 key 当序号用。
       */
      seq: number;
      utterance: Utterance;
      /** 播完之后要不要听候选人回答。 */
      expects_answer: boolean;
      answer: AnswerSettings | null;
      progress: Progress;
      /** 接下来多半会用到的语音地址，页面可以提前取好，减少题与题之间的等待。 */
      preload_audio_urls?: string[];
    }
  | {
      action: "finish";
      /** 结束原因，例如 completed、ended_early。 */
      reason: string;
    };

/** 一段回答是怎么结束的。 */
export type AnswerEndReason =
  /** 静音达到设定时长。 */
  | "silence"
  /** 候选人点了「我答完了」。 */
  | "button"
  /** 到了最长作答时间。 */
  | "timeout"
  /** 一直没有开口。 */
  | "no_speech";

/** 上报刚完成的这一句（以及候选人的回答）。时间都是面试时间轴上的毫秒。 */
export interface AdvanceRequest {
  /** 这一句的序号，取自服务端发来的那一步。 */
  seq: number;
  utterance_key: string;
  playback_start_ms: number;
  playback_end_ms: number;
  /** 候选人第一次开口的时刻；这句话不需要回答，或候选人没开口，是 null。 */
  answer_start_ms: number | null;
  /** 候选人最后停止说话的时刻。 */
  answer_end_ms: number | null;
  end_reason: AnswerEndReason | null;
  /** 重听了几次。 */
  replays: number;
}

export interface ClientEvent {
  kind: string;
  /** 面试时间轴上的毫秒；面试开始前的事件是 null。 */
  at_ms: number | null;
  /** 事件发生的时刻（RFC 3339），用来给面试开始前的事件排序。 */
  occurred_at: string;
  payload: Record<string, unknown>;
}

export interface DeviceCheckReport {
  camera: "ok" | "failed" | "skipped";
  microphone: "ok" | "failed";
  speaker: "ok" | "failed" | "skipped";
  /** 上行测速结果（千比特每秒）；没测是 null。 */
  uplink_kbps: number | null;
  /** 选中的录制格式；浏览器一个都不支持是 null。 */
  recording_mime: string | null;
  user_agent: string;
}

export interface SegmentOpenRequest {
  track: RecordingTrack;
  segment_id: string;
  content_type: string;
  start_ms: number;
}
