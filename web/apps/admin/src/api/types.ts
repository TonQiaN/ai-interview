/**
 * 接口的数据类型。
 *
 * 路径、分页、错误结构和状态机照 docs/architecture/api.md 和 data-model.md 写。
 * 面试配置、题目计划、分析报告这几个 JSON 的精确结构归 AI 引擎线和分析报告线，
 * 它们的类型发布到 @ai-interview/api-client 之后，这个文件换成从那里导入。
 */

import type { Schemas } from "@ai-interview/api-client";

export interface Page<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export interface ApiErrorBody {
  error: { code: string; message: string; details: unknown };
}

/** 这一段的类型直接取自后端的 OpenAPI 文档（@ai-interview/api-client），后端改了字段这里编译就会报错。 */
export type AdminUser = Schemas["AdminUser"];
export type LoginInput = Schemas["LoginRequest"];

// ───────── 岗位 ─────────

export type PositionStatus = Schemas["PositionStatus"];
export type PositionSource = Schemas["PositionSource"];
export type PositionExtra = Schemas["PositionExtra"];
export type Position = Schemas["Position"];

/** 新建和修改岗位时提交的内容 */
export type PositionInput = Schemas["CreatePosition"];

/** 把一段岗位描述交给大模型整理出来的草稿 */
export type PositionDraft = Schemas["PositionDraft"];

// ───────── RAAS 导入：只从 RAAS 读，导进来的是一份独立的副本 ─────────

export type RaasStatus = Schemas["RaasStatus"];
export type RaasPosition = Schemas["RaasPositionItem"];
export type RaasCandidate = Schemas["RaasCandidateItem"];
export type RaasImportOutcome = Schemas["ImportOutcome"];
export type RaasPositionImportItem = Schemas["PositionImportItem"];
export type RaasCandidateImportItem = Schemas["CandidateImportItem"];

// ───────── 候选人和简历 ─────────

export type CandidateSource = Schemas["CandidateSource"];
export type ParseStatus = Schemas["ParseStatus"];

export type CandidateProfile = Schemas["CandidateProfile"];
export type ParsedResume = Schemas["ParsedResume"];
export type DuplicateHint = Schemas["DuplicateHint"];

export type Resume = Schemas["Resume"];

/** 简历详情：多了全文和结构化结果，解析完成后才有 */
export type ResumeDetail = Resume & {
  text_content: string | null;
  interview_text: string | null;
  parsed: ParsedResume | null;
};

export type Candidate = Schemas["Candidate"];
export type CandidateListItem = Candidate & { primary_resume: Resume | null };
export type CandidateDetail = CandidateListItem & { possible_duplicates: DuplicateHint[] };

export type CandidateInput = Schemas["CreateCandidate"];

/** 上传简历建候选人的返回 */
export interface ImportedCandidate {
  candidate: Candidate;
  resume: Resume;
}

// ───────── 语音 ─────────

export type EngineId = "index2" | "index25" | "breeze2";

export interface SpeechEngine {
  id: EngineId;
  name: string;
  languages: string[];
  capabilities: string[];
  requires_reference_audio: boolean;
  requires_reference_transcript: boolean;
  max_text_chars: number | null;
  non_commercial_only: boolean;
  available: boolean;
  unavailable_reason: string | null;
  is_default: boolean;
}

export interface Voice {
  id: string;
  name: string;
  engine: EngineId;
  /** 克隆的音色有参考录音；没有参考录音的是用文字描述设计出来的 */
  cloned: boolean;
  has_reference: boolean;
  reference_duration_ms: number | null;
  reference_transcript: string | null;
  style_prompt: string | null;
  language: string;
  is_default: boolean;
  created_at: string;
}

export interface VoiceInput {
  name: string;
  engine: EngineId;
  cloned: boolean;
  reference_transcript: string | null;
  style_prompt: string | null;
  language: string;
}

// ───────── 面试配置 ─────────

export type Effort = "low" | "high" | "max";
export type InterviewMode = "scripted" | "realtime";

export interface InterviewConfig {
  /** scripted：预先出题逐题问答；realtime：实时对话 */
  mode: InterviewMode;
  language: "zh" | "en";
  expected_duration_min: number;
  /** null 表示按预期时长自动安排 */
  question_count: number | null;
  /** 候选人明显不合适时可以提前结束 */
  allow_early_finish: boolean;
  /** 候选人停顿这么久算这一题答完 */
  silence_sec: number;
  max_answer_sec: number;
  llm: {
    model: string;
    thinking: { enabled: boolean; effort: Effort };
  };
  tts: {
    engine: EngineId;
    voice_id: string | null;
    /** 说话风格，比如「沉稳、亲切」 */
    style: string;
    speed: number;
  };
  /** 注入给面试官的考察重点，一条一句 */
  focus_prompts: string[];
  /** 其他背景：公司介绍、团队情况、这一轮的定位等 */
  extra_context: string;
  record_video: boolean;
}

export interface ConfigDefaults {
  config: InterviewConfig;
  limits: {
    expected_duration_min: { min: number; max: number; step: number };
    question_count: { min: number; max: number };
    silence_sec: { min: number; max: number; presets: number[] };
    max_answer_sec: { min: number; max: number; step: number };
    speed: { min: number; max: number; step: number };
  };
  models: { id: string; name: string }[];
}

export interface InterviewTemplate {
  id: string;
  name: string;
  description: string | null;
  config: InterviewConfig;
  is_default: boolean;
  created_at: string;
  updated_at: string;
}

// ───────── 面试 ─────────

export type InterviewStatus =
  | "draft"
  | "preparing"
  | "prepare_failed"
  | "ready"
  | "invited"
  | "in_progress"
  | "completed"
  | "abandoned"
  | "expired"
  | "cancelled";

export type ReportStatus = "pending" | "running" | "done" | "failed";
/** 报告给的建议：强烈建议推进、建议推进、拿不准、不建议推进 */
export type Recommendation = "strong_advance" | "advance" | "borderline" | "not_advance";
export type Decision = "advance" | "reject" | "undecided";

export interface InterviewSummary {
  id: string;
  status: InterviewStatus;
  candidate: { id: string; name: string; current_title: string | null };
  position: { id: string; title: string; department: string | null };
  mode: InterviewMode;
  expected_duration_min: number;
  started_at: string | null;
  ended_at: string | null;
  duration_ms: number | null;
  end_reason: string | null;
  /** 最新一份报告的状态；还没有报告时为空 */
  report_status: ReportStatus | null;
  overall_score: number | null;
  recommendation: Recommendation | null;
  decision: Decision;
  created_at: string;
  updated_at: string;
}

export interface Interview extends InterviewSummary {
  config: InterviewConfig;
  prepare_error: string | null;
  prepare_progress: { done: number; total: number } | null;
  /** 报告生成失败的原因 */
  report_error: string | null;
  decision_note: string | null;
  decided_at: string | null;
}

export interface InterviewInput {
  candidate_id: string;
  position_id: string;
  config: InterviewConfig;
}

export type UtteranceKind = "opening" | "question" | "follow_up" | "transition" | "closing";
export type SynthStatus = "pending" | "running" | "done" | "failed";

export interface Utterance {
  key: string;
  kind: UtteranceKind;
  text: string;
  /** 这道题想考察什么，只有 HR 看得到 */
  intent: string | null;
  suggested_answer_sec: number | null;
  source: "planned" | "live";
  synth_status: SynthStatus;
  audio_duration_ms: number | null;
}

export interface InterviewPlan {
  questions: {
    key: string;
    text: string;
    intent: string;
    dimension: string;
    suggested_answer_sec: number;
  }[];
  opening: string;
  closing: string;
  estimated_duration_min: number;
}

export type InviteStatus = "active" | "used" | "expired" | "revoked";

export interface Invite {
  id: string;
  status: InviteStatus;
  /** 完整链接只在生成的那一次返回 */
  url: string | null;
  token_hint: string;
  expires_at: string;
  first_opened_at: string | null;
  created_at: string;
}

export interface Recording {
  id: string;
  track: "camera" | "microphone" | "screen";
  mime_type: string;
  start_ms: number;
  duration_ms: number;
  size_bytes: number;
  media_url: string | null;
}

export interface Turn {
  id: string;
  seq: number;
  /** other 是录到的第三个人的声音 */
  speaker: "interviewer" | "candidate" | "other";
  utterance_key: string | null;
  text: string;
  start_ms: number;
  end_ms: number;
  transcript_source: "live" | "final";
}

export interface Transcript {
  turns: Turn[];
  duration_ms: number;
}

export interface InterviewEvent {
  id: string;
  /** visibility_hidden、window_blur、paste、fullscreen_exit、face_count_changed 等，清单见 api.md */
  kind: string;
  /** 面试开始前的事件没有时间轴位置 */
  at_ms: number | null;
  occurred_at: string;
  /** 持续了多久；瞬时事件为空 */
  duration_ms: number | null;
  detail: string | null;
}

// ───────── 分析报告 ─────────
// 照 crates/interview-report/src/report.rs 写（schema_version 1）。
// @ai-interview/api-client 生成出来之后，这一节换成从那里导入。

export type Level = "low" | "medium" | "high";
export type EvidenceSource = "transcript" | "resume" | "event";
export type QuoteMatch = "exact" | "approximate" | "whole_segment" | "unchecked";
export type Speaker = "interviewer" | "candidate" | "other";

export interface Evidence {
  source: EvidenceSource;
  segment_id?: string;
  event_id?: string;
  quote: string;
  /** 在录像时间轴上的位置。简历里的原文没有时间 */
  start_ms?: number;
  end_ms?: number;
  speaker?: Speaker;
  quote_match: QuoteMatch;
}

export interface Finding {
  point: string;
  evidence: Evidence[];
}

export interface ReportRecommendation {
  decision: Recommendation;
  confidence: Level;
  headline: string;
  overview: string;
  key_reasons: Finding[];
  risks: Finding[];
}

export interface CompetencyRating {
  name: string;
  /** 1 到 5 分。没考察到时为空，不是 0 分 */
  rating: number | null;
  comment: string;
  evidence: Evidence[];
}

export type FocusVerdict = "satisfied" | "partially_satisfied" | "not_satisfied" | "not_assessed";

export interface FocusCheck {
  focus: string;
  verdict: FocusVerdict;
  finding: string;
  evidence: Evidence[];
}

export interface QuestionTiming {
  start_ms: number;
  end_ms: number;
  response_latency_ms: number | null;
  candidate_speaking_ms: number;
  candidate_chars: number;
  chars_per_minute: number | null;
  longest_pause_ms: number | null;
}

export interface QuestionJudgement {
  answered: boolean;
  answer_summary: string;
  rating: number;
  rating_reason: string;
  strengths: Finding[];
  weaknesses: Finding[];
  follow_up: string | null;
}

export interface QuestionReview {
  question_id: string;
  question_text: string;
  asked: boolean;
  segment_ids: string[];
  timing: QuestionTiming;
  judgement: QuestionJudgement | null;
}

export type RequirementVerdict = "met" | "partially_met" | "not_met" | "unknown";

export interface RequirementCheck {
  requirement: string;
  importance: "must_have" | "nice_to_have";
  verdict: RequirementVerdict;
  basis: "resume" | "interview" | "both" | "none";
  reasoning: string;
  evidence: Evidence[];
}

export interface RequirementMatch {
  match_level: Level;
  summary: string;
  items: RequirementCheck[];
}

export type Credibility = "high" | "medium" | "low" | "insufficient";
export type ClaimVerdict =
  | "corroborated"
  | "partially_corroborated"
  | "not_covered"
  | "doubtful"
  | "contradicted";

export interface ClaimCheck {
  claim: string;
  verdict: ClaimVerdict;
  reasoning: string;
  evidence: Evidence[];
  follow_up: string | null;
}

export interface ResumeVerification {
  credibility: Credibility;
  summary: string;
  claims: ClaimCheck[];
}

export type RiskLevel = "none" | "low" | "medium" | "high";

export interface IntegritySignal {
  /** scripted_answer、timing_pattern、depth_mismatch、page_event、third_party、identity、manipulation、other */
  kind: string;
  severity: Level;
  reliability: Level;
  description: string;
  alternative_explanations: string;
  evidence: Evidence[];
}

export interface IntegrityAssessment {
  risk_level: RiskLevel;
  summary: string;
  signals: IntegritySignal[];
}

export type CommunicationAspect = "clarity" | "structure" | "responsiveness" | "composure";

export interface AspectRating {
  aspect: CommunicationAspect;
  rating: number | null;
  observation: string;
  evidence: Evidence[];
}

export interface CommunicationAssessment {
  summary: string;
  aspects: AspectRating[];
  reliability_note: string;
}

export interface NextRound {
  focus_areas: string[];
  questions: { question: string; purpose: string }[];
}

export interface TimingOverview {
  duration_ms: number;
  expected_ms: number;
  candidate_speaking_ms: number;
  interviewer_speaking_ms: number;
  questions_planned: number;
  questions_asked: number;
  questions_answered: number;
}

export type ReviewFlagCode =
  | "integrity_risk"
  | "must_have_unmet"
  | "resume_doubt"
  | "weak_evidence"
  | "incomplete_analysis";

export interface ReviewFlag {
  code: ReviewFlagCode;
  message: string;
}

export interface ReportMeta {
  model: string;
  thinking: string;
  prompt_version: number;
  system_fingerprints: string[];
  usage: {
    calls: number;
    prompt_tokens: number;
    completion_tokens: number;
    reasoning_tokens: number;
    cache_hit_tokens: number;
    cache_miss_tokens: number;
  };
  elapsed_ms: number;
  evidence: {
    cited: number;
    exact: number;
    approximate: number;
    whole_segment: number;
    dropped: number;
  };
  failed_stages: { stage: string; error: string }[];
}

export interface InterviewReport {
  schema_version: number;
  interview_id: string;
  generated_at_ms: number;
  recommendation: ReportRecommendation;
  /** 0 到 100。没有任何一题被评分时为空 */
  overall_score: number | null;
  competencies: CompetencyRating[];
  focus_checks: FocusCheck[];
  questions: QuestionReview[];
  requirement_match: RequirementMatch | null;
  resume_verification: ResumeVerification | null;
  integrity: IntegrityAssessment | null;
  communication: CommunicationAssessment | null;
  next_round: NextRound;
  timing: TimingOverview;
  review_flags: ReviewFlag[];
  limitations: string[];
  meta: ReportMeta;
}

// ───────── 系统 ─────────

export type SystemStatus = Schemas["SystemStatus"];
export type ComponentStatus = Schemas["ComponentStatus"];
export type ComponentState = Schemas["ComponentState"];
