//! 接口里出现的类型。

use chrono::{DateTime, Utc};
use interview_core::{
    InterviewDecision, InterviewStatus, ReportStatus, Speaker, SynthStatus, UtteranceKind,
    UtteranceSource,
};
use interview_engine::{ConfigLimits, InterviewConfig, TimeBudget};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// 列表里的一场面试。
#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct InterviewSummary {
    pub id: Uuid,
    pub status: InterviewStatus,
    pub decision: InterviewDecision,
    pub candidate_id: Uuid,
    pub candidate_name: String,
    pub position_id: Uuid,
    pub position_title: String,
    /// 最新一份分析报告的状态。还没有报告时为空
    pub report_status: Option<ReportStatus>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 面试详情里的候选人摘要。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CandidateBrief {
    pub id: Uuid,
    pub name: String,
    pub current_title: Option<String>,
    pub current_company: Option<String>,
}

/// 面试详情里的岗位摘要。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PositionBrief {
    pub id: Uuid,
    pub title: String,
    pub department: Option<String>,
}

/// 一份分析报告的进度。报告本身用 `GET /interviews/{id}/report` 取。
#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct ReportBrief {
    pub id: Uuid,
    /// 第几版。每次重新生成加一
    pub version: i32,
    pub status: ReportStatus,
    /// 生成失败的原因，给人看的。只有 `failed` 时有
    pub error: Option<String>,
    pub generated_at: Option<DateTime<Utc>>,
}

/// 面试官的话合成到哪一步了。
#[derive(Debug, Clone, Copy, Default, Serialize, ToSchema, sqlx::FromRow)]
pub struct SynthProgress {
    /// 一共几句
    pub total: i64,
    /// 已经合成好的
    pub done: i64,
    /// 合成失败的
    pub failed: i64,
}

/// 一场面试的详情。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InterviewDetail {
    pub id: Uuid,
    pub status: InterviewStatus,
    pub candidate: CandidateBrief,
    pub position: PositionBrief,
    /// 出题用哪份简历。为空时用候选人的主简历
    pub resume_id: Option<Uuid>,
    pub template_id: Option<Uuid>,
    /// 面试官音色。为空时用默认音色
    pub voice_id: Option<Uuid>,
    pub config: InterviewConfig,
    /// 按现在的配置大约几道题、各部分多长时间
    pub time_budget: TimeBudget,
    /// 题目出好了没有。题目本身用 `GET /interviews/{id}/plan` 取
    pub has_plan: bool,
    pub synth: SynthProgress,
    /// 准备失败的原因
    pub prepare_error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub end_reason: Option<String>,
    pub decision: InterviewDecision,
    pub decision_note: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    /// 最新一份分析报告
    pub report: Option<ReportBrief>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 列表的筛选条件。分页和排序见公共的分页参数。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InterviewFilter {
    /// 只看这个候选人的
    pub candidate_id: Option<Uuid>,
    /// 只看这个岗位的
    pub position_id: Option<Uuid>,
    /// 只看这个状态的
    pub status: Option<InterviewStatus>,
    /// 带上已归档的。默认不带
    pub include_archived: Option<bool>,
}

/// 新建一场面试。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateInterview {
    pub candidate_id: Uuid,
    pub position_id: Uuid,
    /// 出题用哪份简历。不填用候选人的主简历
    pub resume_id: Option<Uuid>,
    /// 用哪个配置模板。`config` 也填了的话以 `config` 为准
    pub template_id: Option<Uuid>,
    /// 面试官音色。不填用默认音色
    pub voice_id: Option<Uuid>,
    /// 面试配置。不填就用模板的；模板也没有就用默认配置
    pub config: Option<InterviewConfig>,
}

/// 改一场面试。只传要改的字段。
///
/// 换了简历，或者改的配置写进了题目里，已经出好的题目会作废，要重新准备；
/// 改的配置不影响题目时题目和语音都保留；只换音色的话题目保留，只重新合成语音。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateInterview {
    pub config: Option<InterviewConfig>,
    pub resume_id: Option<Uuid>,
    pub voice_id: Option<Uuid>,
}

/// 改完之后的面试。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UpdatedInterview {
    #[serde(flatten)]
    pub interview: InterviewDetail,
    /// 这次改动让已经出好的题目作废了的话，是哪些字段导致的（配置里的路径，比如
    /// `interviewer.name`；换了简历是 `resume_id`）。题目还能用、或者本来就没出题时是空的
    pub replan_reasons: Vec<String>,
}

/// 开始准备时的选项。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct PrepareRequest {
    /// 已经有题目时要不要重新出。默认不重新出，只补上没合成好的语音
    #[serde(default)]
    pub regenerate_plan: bool,
}

/// 面试官要说的一句话。
#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct UtteranceView {
    /// 在这场面试里稳定唯一的名字，例如 `opening`、`q1`、`q1.f1`、`closing`
    pub key: String,
    pub seq: i32,
    pub kind: UtteranceKind,
    pub text: String,
    pub source: UtteranceSource,
    pub synth_status: SynthStatus,
    pub synth_error: Option<String>,
    /// 用哪个 TTS 引擎合成的
    pub tts_engine: Option<String>,
    pub audio_duration_ms: Option<i32>,
    /// 有没有可以试听的音频（`GET /interviews/{id}/utterances/{key}/audio`）
    pub has_audio: bool,
}

/// 配置表单要用的默认值和取值范围。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConfigDefaults {
    /// 默认配置
    pub config: InterviewConfig,
    /// 各项的取值范围和建议的选项
    pub limits: ConfigLimits,
    /// 默认配置下大约几道题、各部分多长时间
    pub time_budget: TimeBudget,
}

/// 面试配置模板。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub config: InterviewConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 新建模板。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateTemplate {
    pub name: String,
    pub description: Option<String>,
    pub config: InterviewConfig,
}

/// 改模板。只传要改的字段。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateTemplate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub config: Option<InterviewConfig>,
}

/// 一个字（或一个英文单词）和它在面试时间轴上的起止毫秒。
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptWord {
    pub w: String,
    pub s: i64,
    pub e: i64,
}

/// 对话记录里的一段：一方连续说的一段话。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TurnView {
    pub id: Uuid,
    pub seq: i32,
    pub speaker: Speaker,
    /// 面试官的话是它自己的 key；候选人的话是它在回答的那一句的 key
    pub utterance_key: Option<String>,
    pub text: String,
    /// 相对面试开始的毫秒，和录像同一条时间轴
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    /// 逐字时间戳。没有时是空的
    pub words: Vec<TranscriptWord>,
    /// 语音识别的置信度，0 到 1
    pub confidence: Option<f32>,
    /// `live` 是面试中实时转写的，`final` 是面试后重新转写的
    pub transcript_source: String,
    /// 候选人这段回答是怎么结束的：`silence`、`button`、`timeout`、`no_speech`
    pub end_reason: Option<String>,
}

/// 带时间戳的对话记录。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TranscriptView {
    pub items: Vec<TurnView>,
}

/// 看哪一版报告。
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReportQuery {
    /// 第几版。不填看最新生成好的那一版
    pub version: Option<i32>,
}

/// 一份生成好的分析报告。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportView {
    pub id: Uuid,
    pub version: i32,
    pub generated_at: Option<DateTime<Utc>>,
    /// 报告内容
    #[schema(value_type = interview_report::InterviewReport)]
    pub content: serde_json::Value,
}

/// 这场面试的所有报告版本，新的在前。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportList {
    pub items: Vec<ReportBrief>,
}

/// HR 的结论。
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DecisionRequest {
    pub decision: InterviewDecision,
    /// 备注，可以不填
    pub note: Option<String>,
}
