//! 分析的输入：一场面试的完整记录。
//!
//! 这些类型是自包含的，不依赖数据库。负责存取的一方把库里的数据组装成 [`InterviewRecord`]，
//! 再交给 [`crate::analyze`]。所有时间都是相对面试开始的毫秒，和录像同一条时间轴。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 一场面试的完整记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewRecord {
    pub interview_id: String,
    pub job: Job,
    pub candidate: Candidate,
    pub config: InterviewConfig,
    /// 计划的题目和实际问出的追问。
    pub questions: Vec<Question>,
    /// 带时间戳的转写，按时间顺序。
    pub transcript: Vec<Segment>,
    /// 候选人网页采集到的过程事件（切出页面、粘贴等）。留空表示采集了但什么都没发生。
    #[serde(default)]
    pub events: Vec<ProctorEvent>,
    /// 面试实际持续的毫秒数。
    pub duration_ms: u64,
    /// 面试是怎么结束的，例如「全部题目答完」「候选人中途离开」「AI 提前结束」。原样交给模型理解。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
}

/// 岗位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Job {
    pub title: String,
    /// 岗位职责和任职要求的原文，不要求结构化。
    pub description: String,
    /// HR 明确标出的硬性要求。留空时由模型从 `description` 里读出要求。
    #[serde(default)]
    pub must_have: Vec<String>,
    /// HR 明确标出的加分项。
    #[serde(default)]
    pub nice_to_have: Vec<String>,
}

/// 候选人。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Candidate {
    pub candidate_id: String,
    /// 简历正文。姓名、手机、邮箱这些身份信息不影响分析，上游能去掉就去掉。
    pub resume: String,
}

/// HR 对这场面试的设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InterviewConfig {
    /// 这一轮的定位，例如「第一轮初筛，后面还有两轮人工面试」。
    pub round: String,
    /// 预期时长（分钟）。
    pub expected_minutes: u32,
    /// HR 注入的重点考察要求，例如「重点看逻辑推理能力」。报告会逐条回应。
    #[serde(default)]
    pub focus_points: Vec<String>,
    /// 给评估者的其他背景，例如「这个岗位急招，宁可多放人进下一轮」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_notes: Option<String>,
}

/// 一道题。追问也是一道题，用 `parent_id` 指向它追的那道题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Question {
    pub id: String,
    pub text: String,
    /// 这道题想考察什么。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// 出题时写下的评分要点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring_guide: Option<String>,
    /// 追问所属的题目 id。计划内的题目留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// 这道题在总分里的权重，缺省为 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// 说话人。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    Interviewer,
    Candidate,
    /// 录到的第三个人的声音。
    Other,
}

impl Speaker {
    pub const fn label(self) -> &'static str {
        match self {
            Speaker::Interviewer => "面试官",
            Speaker::Candidate => "候选人",
            Speaker::Other => "其他人",
        }
    }
}

/// 转写的一个片段：一方连续说的一段话，可以是一句，也可以是一整段回答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Segment {
    /// 稳定的 id，报告里的证据靠它指回转写。
    pub id: String,
    pub speaker: Speaker,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// 这句话属于哪道题。面试引擎知道就填：计划内的题填它的 id；追问填追问自己的 id
    /// （在 `questions` 里用 `parent_id` 挂到它追的题上），或者直接填「题目 id.f序号」这种写法。
    /// 开场白、结束语这些不属于任何题目的话留空或者填别的标记都可以。
    /// 所有片段都没填时，分析时由模型来分段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    /// 语音识别的置信度，0 到 1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// 逐字时间戳。有的话，证据的时间会精确到引文那几个字，而不是整个片段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<Word>,
}

/// 一个字（或一个英文单词）和它的起止时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Word {
    /// 文字。
    pub w: String,
    /// 开始时间（毫秒）。
    pub s: u64,
    /// 结束时间（毫秒）。
    pub e: u64,
}

/// 候选人网页采集到的一个过程事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProctorEvent {
    pub id: String,
    pub at_ms: u64,
    /// 事件种类，例如 `visibility_hidden`、`window_blur`、`paste`、`fullscreen_exit`、`face_count_changed`，
    /// 清单见 `docs/architecture/api.md`。这里不限定取值，新的种类直接加，含义由模型理解。
    pub kind: String,
    /// 持续了多久，瞬时事件留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 补充说明。事件带的数据（payload）转成文字放在这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
