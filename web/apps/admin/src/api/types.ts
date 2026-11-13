/**
 * 接口的数据类型。
 *
 * 都取自后端的 OpenAPI 文档（@ai-interview/api-client），后端改了字段这里编译就会报错。
 * 例外是「语音」一段：音色和试听的接口后端还没有，那一段是照 docs/architecture/api.md
 * 的路径先写的，只在演示数据下能用。
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

export type Effort = Schemas["Effort"];
export type Thinking = Schemas["Thinking"];
export type InterviewMode = Schemas["InterviewMode"];
export type InterviewStage = Schemas["InterviewStage"];
export type Seniority = Schemas["Seniority"];
export type Language = Schemas["Language"];
export type InterviewerStyle = Schemas["InterviewerStyle"];
export type CandidateQuestions = Schemas["CandidateQuestions"];
export type Competency = Schemas["Competency"];
export type InterviewConfig = Schemas["InterviewConfig"];
export type IntRange = Schemas["IntRange"];
export type ConfigLimits = Schemas["ConfigLimits"];
export type TimeBudget = Schemas["TimeBudget"];
export type ConfigDefaults = Schemas["ConfigDefaults"];

export type InterviewTemplate = Schemas["Template"];
export type TemplateInput = Schemas["CreateTemplate"];

// ───────── 面试 ─────────

export type InterviewStatus = Schemas["InterviewStatus"];
export type ReportStatus = Schemas["ReportStatus"];
/** 报告给的建议：强烈建议进入下一轮、建议进入、待定、不建议。 */
export type Recommendation = Schemas["Decision"];
/** HR 做的决定。`pending` 是还没定。 */
export type Decision = Schemas["InterviewDecision"];

export type InterviewSummary = Schemas["InterviewSummary"];
export type Interview = Schemas["InterviewDetail"];
export type InterviewInput = Schemas["CreateInterview"];
export type InterviewPatch = Schemas["UpdateInterview"];
export type ReportBrief = Schemas["ReportBrief"];
export type SynthProgress = Schemas["SynthProgress"];

// ───────── 题目 ─────────

export type InterviewPlan = Schemas["InterviewPlan"];
export type InterviewBrief = Schemas["InterviewBrief"];
export type PlannedQuestion = Schemas["PlannedQuestion"];
export type FollowUp = Schemas["FollowUp"];
export type Phrases = Schemas["Phrases"];
export type QuestionCategory = Schemas["QuestionCategory"];
export type QuestionOrigin = Schemas["QuestionOrigin"];

export type UtteranceKind = Schemas["UtteranceKind"];
export type SynthStatus = Schemas["SynthStatus"];
export type Utterance = Schemas["UtteranceView"];

// ───────── 面试链接、录像、逐字稿、过程事件 ─────────

export type InviteStatus = Schemas["InviteStatus"];
export type Invite = Schemas["Invite"];
export type CreatedInvite = Schemas["CreatedInvite"];

export type RecordingTrack = Schemas["RecordingTrack"];
export type Recording = Schemas["Recording"];

export type Turn = Schemas["TurnView"];

/** 事件的补充信息因种类而异，见 docs/architecture/api.md 的「过程事件」。 */
export type InterviewEvent = Omit<Schemas["InterviewEvent"], "payload"> & {
  payload: Record<string, unknown>;
};

// ───────── 分析报告 ─────────

export type Level = Schemas["Level"];
export type EvidenceSource = Schemas["EvidenceSource"];
export type QuoteMatch = Schemas["QuoteMatch"];
export type Speaker = Schemas["Speaker"];
export type Evidence = Schemas["Evidence"];
export type Finding = Schemas["Finding"];
export type ReportRecommendation = Schemas["Recommendation"];
export type CompetencyRating = Schemas["CompetencyRating"];
export type FocusVerdict = Schemas["FocusVerdict"];
export type FocusCheck = Schemas["FocusCheck"];
export type QuestionTiming = Schemas["QuestionTiming"];
export type QuestionJudgement = Schemas["QuestionJudgement"];
export type QuestionReview = Schemas["QuestionReview"];
export type RequirementVerdict = Schemas["RequirementVerdict"];
export type RequirementCheck = Schemas["RequirementCheck"];
export type RequirementMatch = Schemas["RequirementMatch"];
export type Credibility = Schemas["Credibility"];
export type ClaimVerdict = Schemas["ClaimVerdict"];
export type ClaimCheck = Schemas["ClaimCheck"];
export type ResumeVerification = Schemas["ResumeVerification"];
export type RiskLevel = Schemas["RiskLevel"];
export type IntegritySignal = Schemas["IntegritySignal"];
export type IntegrityAssessment = Schemas["IntegrityAssessment"];
export type CommunicationAspect = Schemas["CommunicationAspect"];
export type AspectRating = Schemas["AspectRating"];
export type CommunicationAssessment = Schemas["CommunicationAssessment"];
export type NextRound = Schemas["NextRound"];
export type TimingOverview = Schemas["TimingOverview"];
export type ReviewFlagCode = Schemas["ReviewFlagCode"];
export type ReviewFlag = Schemas["ReviewFlag"];
export type ReportMeta = Schemas["ReportMeta"];
export type InterviewReport = Schemas["InterviewReport"];
/** 报告接口返回的外壳：版本号、生成时间，报告本身在 `content` 里。 */
export type ReportEnvelope = Schemas["ReportView"];

// ───────── 系统 ─────────

export type SystemStatus = Schemas["SystemStatus"];
export type ComponentStatus = Schemas["ComponentStatus"];
export type ComponentState = Schemas["ComponentState"];
