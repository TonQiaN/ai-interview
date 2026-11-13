//! 分析的输出：给 HR 看的初筛报告。
//!
//! 报告是建议，不是决定。每个结论都带证据，证据指回转写里的具体片段和时间点。
//! 结构变了就把 [`SCHEMA_VERSION`] 加一，HR 后台按版本号决定怎么展示。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::record::Speaker;

/// 报告结构的版本号。
pub const SCHEMA_VERSION: u32 = 1;

/// 一场面试的分析报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewReport {
    pub schema_version: u32,
    pub interview_id: String,
    /// 生成时间，Unix 毫秒。
    pub generated_at_ms: u64,
    /// 是否建议进入下一轮，以及理由。
    pub recommendation: Recommendation,
    /// 综合得分，0 到 100，由各题评分按权重算出（每题 1 到 5 分，乘以 20）。没有任何一题被评分时为空。
    pub overall_score: Option<u8>,
    /// 各项能力的评价。
    pub competencies: Vec<CompetencyRating>,
    /// 对 HR 注入的每条重点考察要求的回应。
    pub focus_checks: Vec<FocusCheck>,
    /// 逐题评价，顺序和面试时一致。
    pub questions: Vec<QuestionReview>,
    /// 和岗位要求的匹配情况。这一步分析失败时为空，原因见 `meta.failed_stages`。
    pub requirement_match: Option<RequirementMatch>,
    /// 简历内容在面试里有没有被印证。
    pub resume_verification: Option<ResumeVerification>,
    /// 作弊和诚信方面的迹象。
    pub integrity: Option<IntegrityAssessment>,
    /// 沟通表达和应对难题时的稳定性。
    pub communication: Option<CommunicationAssessment>,
    /// 给下一轮人工面试官的建议。
    pub next_round: NextRound,
    /// 整场面试的计时数据。
    pub timing: TimingOverview,
    /// 需要人工复核的提示，由程序按固定条件给出。
    pub review_flags: Vec<ReviewFlag>,
    /// 这份报告的局限，例如面试太短、转写质量差。
    pub limitations: Vec<String>,
    pub meta: ReportMeta,
}

/// 是否建议进入下一轮。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// 强烈建议进入下一轮。
    StrongAdvance,
    /// 建议进入下一轮。
    Advance,
    /// 拿不准，建议 HR 看过再定。
    Borderline,
    /// 不建议进入下一轮。
    NotAdvance,
}

/// 三档程度：低、中、高。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}

/// 综合建议。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Recommendation {
    pub decision: Decision,
    /// 对这个建议有多大把握。
    pub confidence: Level,
    /// 一句话结论。
    pub headline: String,
    /// 一段话的总体评价。
    pub overview: String,
    /// 支持这个建议的主要理由。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub key_reasons: Vec<Finding>,
    /// 主要的风险和疑点。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub risks: Vec<Finding>,
}

/// 一个带证据的观察。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Finding {
    pub point: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 证据的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    /// 面试转写里的一句话。
    #[default]
    Transcript,
    /// 简历原文。
    Resume,
    /// 候选人网页采集到的过程事件。
    Event,
}

/// 引文和原文对得上的程度，由程序核对后填写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuoteMatch {
    /// 引文是原文里连续的一段。
    Exact,
    /// 引文和原文大体一致，个别字不同。
    Approximate,
    /// 模型只给了片段 id 没给引文，这里的引文是整个片段的原文。
    WholeSegment,
    /// 还没核对。交给 HR 的报告里不会出现这个值。
    #[default]
    Unchecked,
}

/// 一条证据。模型只负责给出片段 id 和引文，时间和说话人由程序从转写里回填。
/// 核对不上的证据会被丢掉，数量记在 `meta.evidence` 里。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct Evidence {
    #[serde(default)]
    pub source: EvidenceSource,
    /// 转写片段的 id。来源是转写时有值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// 过程事件的 id。来源是过程事件时有值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// 原话。来源是过程事件时是对事件的描述。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub quote: String,
    /// 在录像时间轴上的起点（毫秒），HR 后台用它跳到录像对应位置。简历证据没有时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
    #[serde(default)]
    pub quote_match: QuoteMatch,
}

/// 一项能力的评价。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CompetencyRating {
    /// 能力名称，例如「专业能力」。
    pub name: String,
    /// 1 到 5 分。面试里没考察到时为空。
    pub rating: Option<u8>,
    pub comment: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 重点考察要求的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FocusVerdict {
    Satisfied,
    PartiallySatisfied,
    NotSatisfied,
    /// 面试里没有足够的内容来判断。
    NotAssessed,
}

/// 对 HR 注入的一条重点考察要求的回应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FocusCheck {
    /// HR 写的要求原文。
    pub focus: String,
    pub verdict: FocusVerdict,
    pub finding: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 一道题的评价。追问并入它所追的那道题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct QuestionReview {
    pub question_id: String,
    pub question_text: String,
    /// 这道题有没有被问到。面试提前结束时，后面的题不会被问到，也不参与评分。
    pub asked: bool,
    /// 这道题（含追问）对应的转写片段，按时间顺序。
    pub segment_ids: Vec<String>,
    pub timing: QuestionTiming,
    /// 模型的评价。这道题没被问到或者分析失败时为空。
    pub judgement: Option<QuestionJudgement>,
}

/// 模型对一道题作答的评价。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct QuestionJudgement {
    /// 候选人是否做了实质性的回答。
    pub answered: bool,
    /// 候选人说了什么。
    pub answer_summary: String,
    /// 1 到 5 分：5 优秀，4 良好，3 合格，2 较弱，1 不合格。
    pub rating: u8,
    pub rating_reason: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub strengths: Vec<Finding>,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub weaknesses: Vec<Finding>,
    /// 下一轮值得追问的问题。
    #[serde(default)]
    pub follow_up: Option<String>,
}

/// 一道题的计时数据，由程序从时间戳算出。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct QuestionTiming {
    pub start_ms: u64,
    pub end_ms: u64,
    /// 面试官问完到候选人开口之间的时间。候选人没开口时为空。
    pub response_latency_ms: Option<u64>,
    /// 候选人说话的总时长。
    pub candidate_speaking_ms: u64,
    /// 候选人说的字数。
    pub candidate_chars: u32,
    /// 语速，字每分钟。说话时间太短时为空。
    pub chars_per_minute: Option<u32>,
    /// 候选人作答中间最长的一次停顿。
    pub longest_pause_ms: Option<u64>,
}

/// 岗位要求的重要程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    MustHave,
    NiceToHave,
}

/// 一条岗位要求是否满足。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementVerdict {
    Met,
    PartiallyMet,
    NotMet,
    /// 简历和面试里都没有足够的信息。
    Unknown,
}

/// 判断依据来自哪里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Resume,
    Interview,
    Both,
    None,
}

/// 和岗位要求的匹配情况。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RequirementMatch {
    pub match_level: Level,
    pub summary: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub items: Vec<RequirementCheck>,
}

/// 一条岗位要求的核对结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RequirementCheck {
    pub requirement: String,
    pub importance: Importance,
    pub verdict: RequirementVerdict,
    pub basis: Basis,
    pub reasoning: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 简历整体的可信程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Credibility {
    High,
    Medium,
    Low,
    /// 面试里涉及简历的内容太少，无法判断。
    Insufficient,
}

/// 简历里一条说法的核验结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimVerdict {
    /// 面试里讲出了只有亲历者才讲得出的细节。
    Corroborated,
    /// 讲到了，但细节不够。
    PartiallyCorroborated,
    /// 面试里没问到。
    NotCovered,
    /// 讲得含糊、回避，或者细节和这段经历应有的深度不相称。
    Doubtful,
    /// 面试里说的和简历写的对不上。
    Contradicted,
}

/// 简历真实性核验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResumeVerification {
    pub credibility: Credibility,
    pub summary: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub claims: Vec<ClaimCheck>,
}

/// 简历里一条说法的核验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ClaimCheck {
    /// 简历里的说法。
    pub claim: String,
    pub verdict: ClaimVerdict,
    pub reasoning: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
    /// 下一轮用来核实这一条的问题。
    #[serde(default)]
    pub follow_up: Option<String>,
}

/// 诚信风险的程度。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 没有发现迹象。
    None,
    Low,
    Medium,
    High,
}

/// 诚信迹象的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// 回答像是照着念的：书面化、过于工整，和口语作答不相称。
    ScriptedAnswer,
    /// 开口前的等待、语速等计时上的异常规律。
    TimingPattern,
    /// 泛泛而谈时很流利，问到个人经历的细节就答不上来。
    DepthMismatch,
    /// 候选人网页记录到的事件：切出页面、粘贴、退出全屏等。
    PageEvent,
    /// 有第三个人在场或提示的迹象。
    ThirdParty,
    /// 录像画面里看到的情况：候选人不在画面里、画面被遮住、看得到手机。画面里多出人归 `third_party`。
    VideoFrame,
    /// 面试的人可能不是简历上的人。
    Identity,
    /// 试图对 AI 下指令来影响评价。
    Manipulation,
    #[serde(other)]
    Other,
}

/// 作弊和诚信方面的评估。迹象不等于定论，需要 HR 复核。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IntegrityAssessment {
    pub risk_level: RiskLevel,
    pub summary: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub signals: Vec<IntegritySignal>,
}

/// 一条诚信迹象。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IntegritySignal {
    pub kind: SignalKind,
    /// 如果属实，问题有多严重。
    pub severity: Level,
    /// 这条迹象本身有多可靠。
    pub reliability: Level,
    pub description: String,
    /// 不是作弊也会出现这种现象的其他解释。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub alternative_explanations: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 沟通表现的一个方面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationAspect {
    /// 表达是否清楚。
    Clarity,
    /// 是否有条理。
    Structure,
    /// 是否听懂问题并切题作答。
    Responsiveness,
    /// 遇到难题或不会的问题时是否稳得住。
    Composure,
}

/// 沟通表达和情绪稳定性。只从语言行为判断，不看表情和声音。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommunicationAssessment {
    pub summary: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub aspects: Vec<AspectRating>,
    /// 这部分结论的可靠性说明。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub reliability_note: String,
}

/// 沟通表现一个方面的评价。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AspectRating {
    pub aspect: CommunicationAspect,
    /// 1 到 5 分，无法判断时为空。
    pub rating: Option<u8>,
    pub observation: String,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub evidence: Vec<Evidence>,
}

/// 给下一轮人工面试官的建议。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
pub struct NextRound {
    /// 建议重点考察的方向。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub focus_areas: Vec<String>,
    /// 建议问的问题。
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub questions: Vec<SuggestedQuestion>,
}

/// 建议下一轮问的一个问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SuggestedQuestion {
    pub question: String,
    /// 问这个问题是为了弄清什么。
    pub purpose: String,
}

/// 整场面试的计时数据。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct TimingOverview {
    pub duration_ms: u64,
    pub expected_ms: u64,
    pub candidate_speaking_ms: u64,
    pub interviewer_speaking_ms: u64,
    /// 计划内的题目数（不含追问）。
    pub questions_planned: u32,
    /// 实际问出的题目数。
    pub questions_asked: u32,
    /// 候选人做了实质性回答的题目数。
    pub questions_answered: u32,
}

/// 需要人工复核的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFlagCode {
    /// 诚信风险为中或高。
    IntegrityRisk,
    /// 有硬性要求没满足，但建议是进入下一轮。
    MustHaveUnmet,
    /// 简历里有说法和面试对不上或存疑。
    ResumeDoubt,
    /// 模型给出的证据有较多核对不上。
    WeakEvidence,
    /// 有分析步骤失败，报告不完整。
    IncompleteAnalysis,
}

/// 一条需要人工复核的提示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ReviewFlag {
    pub code: ReviewFlagCode,
    pub message: String,
}

/// 这份报告是怎么生成的。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
pub struct ReportMeta {
    /// 用的模型，例如 `deepseek-flash`。
    pub model: String,
    /// 思考模式：`off`、`low`、`high`、`max`，或者 `default`（用的是模型客户端配置的默认值）。
    pub thinking: String,
    /// 提示词版本。提示词改了就加一，方便对比不同版本的报告。
    pub prompt_version: u32,
    /// 模型服务返回的指纹，别名指向的版本变了它会变。
    pub system_fingerprints: Vec<String>,
    pub usage: Usage,
    /// 分析用了多久。
    pub elapsed_ms: u64,
    pub evidence: EvidenceStats,
    #[serde(default, deserialize_with = "null_as_default")]
    #[schema(required = true)]
    pub failed_stages: Vec<StageFailure>,
}

/// 模型调用的用量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct Usage {
    pub calls: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 输出里用于思考的部分。
    pub reasoning_tokens: u64,
    /// 输入里命中缓存的部分。
    pub cache_hit_tokens: u64,
    /// 输入里没命中缓存的部分。
    pub cache_miss_tokens: u64,
}

/// 证据核对的统计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
pub struct EvidenceStats {
    /// 模型一共给了多少条证据。
    pub cited: u32,
    pub exact: u32,
    pub approximate: u32,
    pub whole_segment: u32,
    /// 核对不上被丢掉的条数。
    pub dropped: u32,
}

/// 失败的分析步骤。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StageFailure {
    pub stage: String,
    pub error: String,
}

/// 模型有时会把空数组、空字符串写成 `null`。这类字段按缺省值收下，不必为此让模型重来一次。
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
