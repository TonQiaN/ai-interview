//! 接口里出现的类型。

use chrono::{DateTime, Utc};
use interview_core::{
    InterviewDecision, InterviewStatus, ReportStatus, SynthStatus, UtteranceKind, UtteranceSource,
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

/// 面试详情里的报告摘要。
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReportBrief {
    pub id: Uuid,
    pub version: i32,
    pub status: ReportStatus,
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
/// 改了配置或简历，已经出好的题目会作废，要重新准备；只换音色的话题目保留，只重新合成语音。
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct UpdateInterview {
    pub config: Option<InterviewConfig>,
    pub resume_id: Option<Uuid>,
    pub voice_id: Option<Uuid>,
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
